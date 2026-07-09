//! The generic dataflow engine: parse with the spec's grammar, run its query,
//! build a scope tree, resolve names lexically, then flag assignments whose
//! value can never be read (`unused-assignment`). Function-local only — every
//! approximation errs toward *not* flagging; see the module docs in [`super`]
//! for the full list of things this deliberately does not model.

use std::collections::{HashMap, HashSet};

use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, QueryCursor};

use super::spec::{Cap, LanguageSpec};
use super::UNUSED_ASSIGNMENT_ID;
use crate::finding::{Finding, Severity};

/// What kind of region a scope is. `Function` is the unit of analysis; only
/// variables owned by a `Function` unit are ever flagged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ScopeKind {
    Module,
    Block,
    Function,
    Opaque,
}

struct Scope {
    start: usize,
    end: usize,
    kind: ScopeKind,
    parent: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteKind {
    /// The initializing write of a `@def` (declaration with a value). Flaggable.
    Init,
    /// A plain reassignment (`@assign`). Flaggable.
    Assign,
    /// Read-then-write (`@assign.update`) or pattern write (`@assign.multi`).
    /// Kills earlier values but is never flagged itself.
    NonFlag,
}

/// A binding occurrence after query-capture dedup and pattern extraction.
struct Def<'t> {
    name: &'t str,
    node: Node<'t>,
    /// This def's init is a flaggable write (`@def` — declaration with value).
    has_value: bool,
    /// Hoisted to the scope enclosing its nearest scope, visible from the top.
    hoist: bool,
    /// Byte offset from which refs resolve to this def.
    visible_from: usize,
    /// End of the initializer (the init write's effect point).
    effect_end: usize,
    /// Scope node this binding was explicitly paired with, if any.
    scope_override: Option<Node<'t>>,
}

/// A write occurrence (`@assign*`), pre-resolution.
struct RawWrite<'t> {
    name: &'t str,
    node: Node<'t>,
    kind: WriteKind,
    effect_end: usize,
}

struct Write {
    start: usize,
    end: usize,
    effect_end: usize,
    kind: WriteKind,
    line: usize,
    col: usize,
}

struct Read {
    /// Byte position the read happens at.
    pos: usize,
    /// Byte range of the reading node, for loop containment.
    start: usize,
    end: usize,
}

struct Var {
    name: String,
    /// Owning unit (scope index of kind Function/Opaque/Module).
    unit: usize,
    writes: Vec<Write>,
    reads: Vec<Read>,
    /// Never flag this variable (it escapes into a nested function, is
    /// declared `global`, shares its name with unresolvable reads, ...).
    masked: bool,
}

pub(super) fn run(spec: &LanguageSpec, text: &str, path: &str) -> Vec<Finding> {
    let mut parser = Parser::new();
    if parser.set_language(&spec.language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    // A file that doesn't parse cleanly is skipped entirely: a broken tree
    // would mis-scope everything, and not flagging beats guessing.
    if root.has_error() {
        return Vec::new();
    }

    let collected = collect(spec, text, root);
    let (scopes, by_node) = build_scopes(text.len(), &collected.scope_nodes);
    flag(spec, path, &collected, &scopes, &by_node)
}

/// Everything the query pass produces, before scope/name resolution.
struct Collected<'t> {
    /// Scope-introducing nodes, deduped with Function/Opaque beating Block.
    scope_nodes: Vec<(Node<'t>, ScopeKind)>,
    defs: Vec<Def<'t>>,
    writes: Vec<RawWrite<'t>>,
    /// Reads: (name, position, containing byte range).
    reads: Vec<(&'t str, usize, usize, usize)>,
    loops: Vec<(usize, usize)>,
    branches: Vec<(usize, usize)>,
    /// `global`/`nonlocal` names with the node they appear at.
    escapes: Vec<(&'t str, Node<'t>)>,
}

/// Dedup priority when one node carries several captures (e.g. an identifier
/// matched by both `@def` and the broad `(identifier) @ref` pattern).
fn prio(cap: Cap) -> u8 {
    match cap {
        Cap::Def => 8,
        Cap::DefBare => 7,
        Cap::DefParam => 6,
        Cap::DefHoist => 5,
        Cap::DefPattern => 4,
        Cap::Assign => 3,
        Cap::AssignUpdate => 2,
        Cap::AssignMulti => 1,
        _ => 0,
    }
}

/// One classified binding node, pre-dedup.
struct Bind<'t> {
    cap: Cap,
    node: Node<'t>,
    effect_end: usize,
    scope_override: Option<Node<'t>>,
}

fn collect<'t>(spec: &LanguageSpec, text: &'t str, root: Node<'t>) -> Collected<'t> {
    let mut binds: HashMap<usize, (u8, Bind<'t>)> = HashMap::new();
    let mut scope_kinds: HashMap<usize, (Node<'t>, ScopeKind)> = HashMap::new();
    let mut raw_refs: Vec<Node<'t>> = Vec::new();
    let mut ignore_ids: HashSet<usize> = HashSet::new();
    let mut ignore_ranges: Vec<(usize, usize)> = Vec::new();
    let mut extra_reads: Vec<(&'t str, usize, usize, usize)> = Vec::new();
    let mut loops = Vec::new();
    let mut branches = Vec::new();
    let mut escapes = Vec::new();

    fn upsert_scope<'t>(
        scope_kinds: &mut HashMap<usize, (Node<'t>, ScopeKind)>,
        node: Node<'t>,
        kind: ScopeKind,
    ) {
        let entry = scope_kinds.entry(node.id()).or_insert((node, kind));
        if entry.1 == ScopeKind::Block {
            entry.1 = kind;
        }
    }

    fn upsert_bind<'t>(binds: &mut HashMap<usize, (u8, Bind<'t>)>, p: u8, bind: Bind<'t>) {
        match binds.get(&bind.node.id()) {
            Some((existing, _)) if *existing >= p => {}
            _ => {
                binds.insert(bind.node.id(), (p, bind));
            }
        }
    }

    /// Extract the bound identifiers of a pattern subtree and record each.
    fn bind_pattern<'t>(
        binds: &mut HashMap<usize, (u8, Bind<'t>)>,
        spec: &LanguageSpec,
        cap: Cap,
        node: Node<'t>,
        scope_override: Option<Node<'t>>,
    ) {
        let mut idents = Vec::new();
        extract_bindings(spec, node, &mut idents);
        for ident in idents {
            let bind = Bind {
                cap,
                node: ident,
                effect_end: ident.end_byte(),
                scope_override,
            };
            upsert_bind(binds, prio(cap), bind);
        }
    }

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&spec.query, root, text.as_bytes());
    while let Some(m) = matches.next() {
        let anchor_end = m
            .captures
            .iter()
            .find(|c| spec.caps[c.index as usize] == Cap::Anchor)
            .map(|c| c.node.end_byte());
        let scope_override = m
            .captures
            .iter()
            .find(|c| spec.caps[c.index as usize] == Cap::PatternScope)
            .map(|c| c.node);
        if let Some(s) = scope_override {
            upsert_scope(&mut scope_kinds, s, ScopeKind::Block);
        }
        for c in m.captures.iter() {
            let cap = spec.caps[c.index as usize];
            let node = c.node;
            match cap {
                Cap::Ref => raw_refs.push(node),
                Cap::Ignore => {
                    if spec.binding_kinds.contains(&node.kind()) {
                        ignore_ids.insert(node.id());
                    } else {
                        ignore_ranges.push((node.start_byte(), node.end_byte()));
                    }
                }
                Cap::Def | Cap::DefBare | Cap::DefHoist | Cap::Assign | Cap::AssignUpdate => {
                    let effect_end = anchor_end.unwrap_or_else(|| node.end_byte());
                    let bind = Bind {
                        cap,
                        node,
                        effect_end,
                        scope_override,
                    };
                    upsert_bind(&mut binds, prio(cap), bind);
                }
                Cap::DefParam | Cap::DefPattern | Cap::AssignMulti => {
                    bind_pattern(&mut binds, spec, cap, node, scope_override);
                }
                Cap::ScopeBlock => upsert_scope(&mut scope_kinds, node, ScopeKind::Block),
                Cap::ScopeFunction => upsert_scope(&mut scope_kinds, node, ScopeKind::Function),
                Cap::ScopeOpaque => upsert_scope(&mut scope_kinds, node, ScopeKind::Opaque),
                Cap::Loop => {
                    loops.push((node.start_byte(), node.end_byte()));
                    branches.push((node.start_byte(), node.end_byte()));
                }
                Cap::Branch => branches.push((node.start_byte(), node.end_byte())),
                Cap::Escape => escapes.push((&text[node.byte_range()], node)),
                Cap::StringInterp => interp_names(text, node, &mut extra_reads),
                Cap::Anchor | Cap::PatternScope => {}
            }
        }
    }

    // Final event lists. A node that carries a def/assign capture is not a
    // read; ignored identifiers and reads inside ignored ranges drop.
    let mut reads: Vec<(&str, usize, usize, usize)> = Vec::new();
    let mut seen_refs: HashSet<usize> = HashSet::new();
    for node in raw_refs {
        let id = node.id();
        if binds.contains_key(&id) || ignore_ids.contains(&id) || !seen_refs.insert(id) {
            continue;
        }
        let (s, e) = (node.start_byte(), node.end_byte());
        if ignore_ranges.iter().any(|&(rs, re)| rs <= s && e <= re) {
            continue;
        }
        reads.push((&text[node.byte_range()], s, s, e));
    }
    reads.append(&mut extra_reads);

    let mut defs = Vec::new();
    let mut writes = Vec::new();
    for (_, (_, b)) in binds {
        let name = &text[b.node.byte_range()];
        match b.cap {
            Cap::Def | Cap::DefBare | Cap::DefParam | Cap::DefPattern | Cap::DefHoist => {
                let hoist = b.cap == Cap::DefHoist;
                defs.push(Def {
                    name,
                    node: b.node,
                    has_value: b.cap == Cap::Def,
                    hoist,
                    visible_from: if hoist { 0 } else { b.effect_end },
                    effect_end: b.effect_end,
                    scope_override: b.scope_override,
                });
            }
            Cap::Assign | Cap::AssignUpdate | Cap::AssignMulti => {
                if b.cap == Cap::AssignUpdate {
                    // A read of the old value happens before the write.
                    let (s, e) = (b.node.start_byte(), b.node.end_byte());
                    reads.push((name, s, s, e));
                }
                writes.push(RawWrite {
                    name,
                    node: b.node,
                    kind: if b.cap == Cap::Assign {
                        WriteKind::Assign
                    } else {
                        WriteKind::NonFlag
                    },
                    effect_end: b.effect_end,
                });
            }
            _ => {}
        }
    }

    Collected {
        scope_nodes: scope_kinds.into_values().collect(),
        defs,
        writes,
        reads,
        loops,
        branches,
        escapes,
    }
}

/// Recursively pull binding identifiers out of a pattern subtree, skipping
/// children under the spec's skip fields (type annotations, default values —
/// their identifiers are reads, not bindings).
fn extract_bindings<'t>(spec: &LanguageSpec, node: Node<'t>, out: &mut Vec<Node<'t>>) {
    if spec.binding_kinds.contains(&node.kind()) {
        out.push(node);
        return;
    }
    for i in 0..node.child_count() as u32 {
        let Some(child) = node.child(i) else { continue };
        if !child.is_named() {
            continue;
        }
        if let Some(field) = node.field_name_for_child(i) {
            if spec.skip_fields.contains(&field) {
                continue;
            }
        }
        extract_bindings(spec, child, out);
    }
}

/// Scan a string literal's text for `{name}` / `{name:spec}` interpolations
/// (Rust `format!`-family). `{{` is an escaped brace. Over-matching in plain
/// strings only adds phantom reads, which can only suppress findings.
fn interp_names<'t>(text: &'t str, node: Node<'t>, out: &mut Vec<(&'t str, usize, usize, usize)>) {
    let s = &text[node.byte_range()];
    let base = node.start_byte();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            i += 2;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        let named = j > start && (bytes[start].is_ascii_alphabetic() || bytes[start] == b'_');
        if named && j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b':') {
            out.push((&s[start..j], base + start, base + start, base + j));
        }
        i = j.max(i + 1);
    }
}

/// Build the scope tree: an implicit module root covering the whole file, plus
/// every captured scope node, parented by byte containment. Returns the scopes
/// and a scope-node-id → scope-index map.
fn build_scopes(
    len: usize,
    scope_nodes: &[(Node, ScopeKind)],
) -> (Vec<Scope>, HashMap<usize, usize>) {
    let mut entries: Vec<(usize, usize, ScopeKind, usize)> = scope_nodes
        .iter()
        .map(|(n, k)| (n.start_byte(), n.end_byte(), *k, n.id()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    let mut scopes = vec![Scope {
        start: 0,
        end: len,
        kind: ScopeKind::Module,
        parent: None,
    }];
    let mut by_node: HashMap<usize, usize> = HashMap::new();
    let mut stack: Vec<usize> = vec![0];
    for (start, end, kind, node_id) in entries {
        while let Some(&top) = stack.last() {
            if scopes[top].start <= start && end <= scopes[top].end {
                break;
            }
            stack.pop();
        }
        let parent = stack.last().copied();
        scopes.push(Scope {
            start,
            end,
            kind,
            parent: parent.or(Some(0)),
        });
        let idx = scopes.len() - 1;
        by_node.insert(node_id, idx);
        stack.push(idx);
    }
    (scopes, by_node)
}

/// Innermost scope containing `node`, by walking its tree ancestors.
fn scope_of(node: Node, by_node: &HashMap<usize, usize>) -> usize {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if let Some(&idx) = by_node.get(&n.id()) {
            return idx;
        }
        cur = n.parent();
    }
    0
}

/// Nearest enclosing Function/Opaque/Module scope, starting at `scope` itself.
fn owning_unit(scopes: &[Scope], scope: usize) -> usize {
    let mut s = scope;
    loop {
        match scopes[s].kind {
            ScopeKind::Function | ScopeKind::Opaque | ScopeKind::Module => return s,
            ScopeKind::Block => s = scopes[s].parent.unwrap_or(0),
        }
    }
}

/// Innermost scope containing a byte range (for synthetic reads with no node).
fn scope_by_pos(scopes: &[Scope], start: usize, end: usize) -> usize {
    let mut best = 0;
    let mut best_size = usize::MAX;
    for (i, s) in scopes.iter().enumerate() {
        if s.start <= start && end <= s.end && s.end - s.start < best_size {
            best = i;
            best_size = s.end - s.start;
        }
    }
    best
}

fn contains(range: (usize, usize), start: usize, end: usize) -> bool {
    range.0 <= start && end <= range.1
}

/// Indices of the branch ranges containing a byte range.
fn branch_set(branches: &[(usize, usize)], start: usize, end: usize) -> HashSet<usize> {
    branches
        .iter()
        .enumerate()
        .filter(|&(_, &b)| contains(b, start, end))
        .map(|(i, _)| i)
        .collect()
}

/// Per-scope name table: (visible_from, var index), sorted by visibility.
type NameTable = HashMap<(usize, String), Vec<(usize, usize)>>;

/// Resolve `name` used at byte `pos` from `start_scope` outward. Merged-vars
/// languages have one variable per (scope, name); otherwise the latest def
/// visible at `pos` wins, falling back to the earliest (use-before-decl stays
/// attributed — conservatively — to the in-scope binding). `stop_after`
/// bounds the walk (Python write locality).
fn resolve(
    spec: &LanguageSpec,
    scopes: &[Scope],
    table: &NameTable,
    name: &str,
    pos: usize,
    start_scope: usize,
    stop_after: Option<usize>,
) -> Option<usize> {
    let mut s = start_scope;
    loop {
        if let Some(cands) = table.get(&(s, name.to_string())) {
            if !cands.is_empty() {
                if spec.merged_vars {
                    return Some(cands[0].1);
                }
                let visible = cands.iter().rev().find(|(vf, _)| *vf <= pos);
                return Some(visible.unwrap_or(&cands[0]).1);
            }
        }
        if Some(s) == stop_after {
            return None;
        }
        s = scopes[s].parent?;
    }
}

/// Get or create the implicit function-local a merged-vars (Python) write
/// targets when no binding is visible inside its unit.
fn implicit_var(vars: &mut Vec<Var>, table: &mut NameTable, unit: usize, name: &str) -> usize {
    let key = (unit, name.to_string());
    if let Some(&(_, idx)) = table.get(&key).and_then(|c| c.first()) {
        return idx;
    }
    vars.push(Var {
        name: name.to_string(),
        unit,
        writes: Vec::new(),
        reads: Vec::new(),
        masked: false,
    });
    let idx = vars.len() - 1;
    table.entry(key).or_default().push((0, idx));
    idx
}

/// Whether write `i` of `var` (writes sorted by position) is provably dead.
/// `Some(None)` — the value is never read after the write at all;
/// `Some(Some(line))` — an unconditional later write at `line` kills it first;
/// `None` — not provably dead, don't flag.
fn dead_reason(c: &Collected, var: &Var, i: usize) -> Option<Option<usize>> {
    let w = &var.writes[i];
    if w.kind == WriteKind::NonFlag {
        return None;
    }
    let next = var.writes.get(i + 1);
    // A read between this write's effect and the next write's effect
    // (unbounded when this is the last write) observes the value.
    let read_in_window = var
        .reads
        .iter()
        .any(|r| r.pos >= w.effect_end && next.is_none_or(|n| r.pos < n.effect_end));
    // Back edges: any read of the variable inside a loop that also contains
    // this write may observe it on a later iteration.
    let loop_reachable = c.loops.iter().any(|&l| {
        contains(l, w.start, w.end) && var.reads.iter().any(|r| contains(l, r.start, r.end))
    });
    if read_in_window || loop_reachable {
        return None;
    }
    if !var.reads.iter().any(|r| r.pos >= w.effect_end) {
        return Some(None); // never read at all after this write
    }
    // Dead only because a later write kills it first — and only provable when
    // that write is not more conditional than this one (it must run on every
    // path that runs this write).
    let n = next?;
    branch_set(&c.branches, n.start, n.end)
        .is_subset(&branch_set(&c.branches, w.start, w.end))
        .then_some(Some(n.line))
}

fn flag(
    spec: &LanguageSpec,
    path: &str,
    c: &Collected,
    scopes: &[Scope],
    by_node: &HashMap<usize, usize>,
) -> Vec<Finding> {
    let mut vars: Vec<Var> = Vec::new();
    let mut table: NameTable = HashMap::new();

    // Place defs. merged_vars (Python) folds every binding of a name in one
    // scope into a single variable; otherwise each def is its own (shadowing).
    for def in &c.defs {
        let scope = match def.scope_override {
            Some(s) => by_node.get(&s.id()).copied().unwrap_or(0),
            None => {
                let s = scope_of(def.node, by_node);
                if def.hoist {
                    scopes[s].parent.unwrap_or(0)
                } else {
                    s
                }
            }
        };
        let key = (scope, def.name.to_string());
        let cands = table.entry(key).or_default();
        let var_idx = if spec.merged_vars && !cands.is_empty() {
            cands[0].1
        } else {
            vars.push(Var {
                name: def.name.to_string(),
                unit: owning_unit(scopes, scope),
                writes: Vec::new(),
                reads: Vec::new(),
                masked: false,
            });
            let idx = vars.len() - 1;
            cands.push((def.visible_from, idx));
            cands.sort_unstable_by_key(|e| e.0);
            idx
        };
        if def.has_value {
            let p = def.node.start_position();
            vars[var_idx].writes.push(Write {
                start: def.node.start_byte(),
                end: def.node.end_byte(),
                effect_end: def.effect_end,
                kind: WriteKind::Init,
                line: p.row + 1,
                col: p.column + 1,
            });
        }
    }

    // `global`/`nonlocal` names, keyed by the unit that declares them.
    let escaped: HashSet<(usize, String)> = c
        .escapes
        .iter()
        .map(|(name, node)| {
            let unit = owning_unit(scopes, scope_of(*node, by_node));
            (unit, name.to_string())
        })
        .collect();

    // Attach writes. Python creates implicit function-locals for writes with
    // no binding visible inside the unit; elsewhere unresolved writes are
    // dropped (flagging requires a resolved variable, so dropping can't flag).
    let mut pending: Vec<(usize, Write, usize)> = Vec::new();
    for w in &c.writes {
        let scope = scope_of(w.node, by_node);
        let unit = owning_unit(scopes, scope);
        let pos = w.node.start_byte();
        let target = if spec.merged_vars {
            match resolve(spec, scopes, &table, w.name, pos, scope, Some(unit)) {
                Some(v) => Some(v),
                None if escaped.contains(&(unit, w.name.to_string())) => scopes[unit]
                    .parent
                    .and_then(|p| resolve(spec, scopes, &table, w.name, pos, p, None)),
                None => Some(implicit_var(&mut vars, &mut table, unit, w.name)),
            }
        } else {
            resolve(spec, scopes, &table, w.name, pos, scope, None)
        };
        if let Some(v) = target {
            let p = w.node.start_position();
            pending.push((
                v,
                Write {
                    start: w.node.start_byte(),
                    end: w.node.end_byte(),
                    effect_end: w.effect_end,
                    kind: w.kind,
                    line: p.row + 1,
                    col: p.column + 1,
                },
                unit,
            ));
        }
    }
    for (v, w, unit) in pending {
        if unit != vars[v].unit {
            vars[v].masked = true; // written from a nested function
        }
        vars[v].writes.push(w);
    }

    // Attach reads. A name that resolves nowhere masks every same-named
    // variable in the file: we can't prove those reads observe none of them.
    let mut loose: HashSet<&str> = HashSet::new();
    for &(name, pos, start, end) in &c.reads {
        let scope = scope_by_pos(scopes, start, end);
        match resolve(spec, scopes, &table, name, pos, scope, None) {
            Some(v) => {
                if owning_unit(scopes, scope) != vars[v].unit {
                    vars[v].masked = true; // read from a nested function
                }
                vars[v].reads.push(Read { pos, start, end });
            }
            None => {
                loose.insert(name);
            }
        }
    }
    for var in &mut vars {
        if loose.contains(var.name.as_str()) || escaped.contains(&(var.unit, var.name.clone())) {
            var.masked = true;
        }
    }

    // The check: flag an Init/Assign write whose value no read can observe.
    let mut findings = Vec::new();
    for var in &mut vars {
        if var.masked || scopes[var.unit].kind != ScopeKind::Function || var.name.starts_with('_') {
            continue;
        }
        var.writes.sort_unstable_by_key(|w| w.start);
        for i in 0..var.writes.len() {
            let Some(reassigned_at) = dead_reason(c, var, i) else {
                continue;
            };
            let w = &var.writes[i];
            let message = match reassigned_at {
                Some(line) => format!(
                    "`{}` is assigned a value that is never read — it is reassigned at line {line} first; drop or use the earlier assignment.",
                    var.name
                ),
                None => format!(
                    "`{}` is assigned a value that is never read — drop the assignment or use the value.",
                    var.name
                ),
            };
            findings.push(Finding {
                rule: UNUSED_ASSIGNMENT_ID.to_string(),
                path: path.to_string(),
                line: w.line,
                col: w.col,
                matched: var.name.clone(),
                message,
                severity: Severity::Warning,
            });
        }
    }
    findings.sort_unstable_by_key(|f| (f.line, f.col));
    findings
}
