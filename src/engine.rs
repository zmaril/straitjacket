//! The scan engine: holds the line rules and a single `RegexSet` built from the
//! regex-backed rules' prefilters. For each line it asks the `RegexSet` which
//! patterns could match (one pass over the line for all of them), then runs only
//! those rules' full scan plus any non-regex rules (the emoji scan). This is the
//! "match many patterns at once" shape, in pure Rust.
//!
//! On top of the per-line rules it also enforces one whole-file rule, `file-size`,
//! which doesn't fit the line-based [`Rule`] trait — it's evaluated from the line
//! count after the per-line pass.

use anyhow::Result;
use regex::RegexSet;

use crate::config::Config;
use crate::finding::{Finding, Severity};
use crate::nesting::{self, DEEP_NESTING_ID, NEST_EXTS};
use crate::react::{
    self, ComponentIndex, EFFECT_ID, ONE_COMPONENT_ID, PROP_DRILLING_ID, REACT_EXTS,
    STORE_PASSTHROUGH_ID,
};
use crate::rules::{line_rules, LineHit, Rule};
use crate::slop_prose::{SlopProse, PROSE_EXTS};

/// Line-scoped escape hatch. `straitjacket-allow` on a line suppresses every rule
/// for that line; `straitjacket-allow:<id>` suppresses only the named rule.
const ALLOW: &str = "straitjacket-allow";

/// Whole-file escape hatch. `straitjacket-allow-file` anywhere in a file suppresses
/// every rule for the file; `straitjacket-allow-file:<id>` suppresses one rule. This
/// is how you exempt, say, a palette file from `color` without per-line markers.
const ALLOW_FILE: &str = "straitjacket-allow-file";

/// Id of the whole-file line-count rule.
const FILE_SIZE_ID: &str = "file-size";

/// Id of the whole-text prose analyzer.
const SLOP_PROSE_ID: &str = "slop-prose";

/// Id of the cross-file copy/paste detector.
const DUPLICATION_ID: &str = "duplication";

/// Id of the synthetic rule that reports a suppression marker which suppressed nothing
/// — straitjacket's analogue of clippy's unused `#[allow]`.
pub const UNUSED_MARKER_ID: &str = "unused-marker";

/// Extensions the `file-size` rule applies to — source, config and docs where a
/// huge single file is a smell (not lockfiles, data dumps, or binaries).
const SIZE_EXTS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "css", "scss", "sass", "less", "vue", "svelte", "html",
    "htm", "py", "rb", "go", "rs", "java", "kt", "kts", "swift", "c", "h", "cc", "cpp", "hpp",
    "cs", "php", "sh", "bash", "zsh", "sql", "md", "markdown", "mdx", "json", "yaml", "yml",
    "toml",
];

pub struct Engine {
    rules: Vec<Box<dyn Rule>>,
    /// `RegexSet` over the prefilter patterns of the regex-backed rules.
    set: RegexSet,
    /// `set` pattern index → index into `rules`.
    set_to_rule: Vec<usize>,
    /// Indices into `rules` of rules with no prefilter (always run, e.g. emoji).
    nonregex: Vec<usize>,
    /// Per-rule enable flag, parallel to `rules`.
    enabled: Vec<bool>,
    /// `file-size` line budget, and whether the rule is enabled.
    max_lines: usize,
    file_size_enabled: bool,
    /// `deep-nesting` indentation-depth budget, and whether the rule is enabled.
    max_nesting: usize,
    nesting_enabled: bool,
    /// The `slop-prose` analyzer, present only when enabled.
    slop_prose: Option<SlopProse>,
    /// `Some(min_tokens)` when the cross-file `duplication` rule is enabled. Run by
    /// the caller (it's a whole-run analysis over the scan paths), not per file.
    duplication: Option<usize>,
    /// Skip `.json` files entirely (generated/config data, not human-written).
    skip_json: bool,
    /// React AST rules, enabled independently.
    one_component: bool,
    effect_in_component: bool,
    prop_drilling: bool,
    store_passthrough: bool,
    /// Cross-file component index for the forwarding rules; set by the caller after
    /// collecting the file list (the rules need every local component's props).
    component_index: Option<ComponentIndex>,
}

impl Engine {
    /// Build an engine from a [`Config`].
    pub fn new(config: &Config) -> Result<Self> {
        let rules = line_rules();
        let mut patterns = Vec::new();
        let mut set_to_rule = Vec::new();
        let mut nonregex = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            match rule.prefilter() {
                Some(p) => {
                    patterns.push(p.to_string());
                    set_to_rule.push(i);
                }
                None => nonregex.push(i),
            }
        }
        let set = RegexSet::new(&patterns)?;
        let enabled = vec![true; rules.len()];
        Ok(Self {
            rules,
            set,
            set_to_rule,
            nonregex,
            enabled,
            max_lines: config.max_lines.unwrap_or(usize::MAX),
            file_size_enabled: config.max_lines.is_some(),
            max_nesting: config.max_nesting.unwrap_or(usize::MAX),
            nesting_enabled: config.max_nesting.is_some(),
            slop_prose: config
                .slop_prose
                .then(|| SlopProse::new(config.prose_window)),
            duplication: config.duplication.then_some(config.dup_min_tokens),
            skip_json: config.skip_json,
            one_component: config.one_component,
            effect_in_component: config.effect_in_component,
            prop_drilling: config.prop_drilling,
            store_passthrough: config.store_passthrough,
            component_index: None,
        })
    }

    /// Whether the forwarding rules are on (so the caller knows to build & set the
    /// cross-file component index).
    pub fn needs_component_index(&self) -> bool {
        self.prop_drilling || self.store_passthrough
    }

    /// Provide the cross-file component index the forwarding rules consult.
    pub fn set_component_index(&mut self, index: ComponentIndex) {
        self.component_index = Some(index);
    }

    /// Min-token threshold if the cross-file `duplication` rule is enabled.
    pub fn duplication(&self) -> Option<usize> {
        self.duplication
    }

    /// Whether `.json` files are skipped (so the caller can mirror it for the
    /// cross-file duplication pass).
    pub fn skip_json(&self) -> bool {
        self.skip_json
    }

    /// Extensions the caller should not read at all under the current config.
    fn ext_skipped(&self, ext: &str) -> bool {
        self.skip_json && ext == "json"
    }

    /// All rule ids, ending with the whole-file / whole-run rules that are enabled.
    pub fn rule_ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<&'static str> = self.rules.iter().map(|r| r.id()).collect();
        ids.push(FILE_SIZE_ID);
        ids.push(DEEP_NESTING_ID);
        if self.slop_prose.is_some() {
            ids.push(SLOP_PROSE_ID);
        }
        if self.duplication.is_some() {
            ids.push(DUPLICATION_ID);
        }
        if self.one_component {
            ids.push(ONE_COMPONENT_ID);
        }
        if self.effect_in_component {
            ids.push(EFFECT_ID);
        }
        if self.prop_drilling {
            ids.push(PROP_DRILLING_ID);
        }
        if self.store_passthrough {
            ids.push(STORE_PASSTHROUGH_ID);
        }
        ids
    }

    /// Human-facing description for a rule id, if present.
    pub fn message_for(&self, id: &str) -> Option<String> {
        match id {
            FILE_SIZE_ID => Some(format!(
                "file longer than {} lines — sprawling single files are a common LLM tell; split it up.",
                self.max_lines
            )),
            DEEP_NESTING_ID => Some(format!(
                "code nested deeper than {} levels — deeply nested logic is hard to follow; extract or flatten it.",
                self.max_nesting
            )),
            SLOP_PROSE_ID => Some(
                "prose that reads like LLM output — machine artifacts hard-fail; a high density of style tells warns/fails.".to_string(),
            ),
            DUPLICATION_ID => Some(format!(
                "copy/pasted code — any clone of {}+ tokens fails; a structure may appear only once.",
                self.duplication.unwrap_or(0)
            )),
            ONE_COMPONENT_ID => {
                Some("more than one React component in a .tsx/.jsx file.".to_string())
            }
            EFFECT_ID => {
                Some("useEffect in a file with a component — extract it to a named hook.".to_string())
            }
            PROP_DRILLING_ID => Some(
                "a prop forwarded unchanged into a child component and never used here — a pure conduit; lift it into a store or context.".to_string(),
            ),
            STORE_PASSTHROUGH_ID => Some(
                "a store value passed unchanged into a child component — have the child read the store directly.".to_string(),
            ),
            _ => self
                .rules
                .iter()
                .find(|r| r.id() == id)
                .map(|r| r.message().to_string()),
        }
    }

    /// Restrict to exactly `ids` (others disabled). Returns ids that don't exist.
    pub fn keep_only(&mut self, ids: &[String]) -> Vec<String> {
        for (i, rule) in self.rules.iter().enumerate() {
            self.enabled[i] = ids.iter().any(|id| id == rule.id());
        }
        self.file_size_enabled = self.file_size_enabled && ids.iter().any(|id| id == FILE_SIZE_ID);
        self.nesting_enabled = self.nesting_enabled && ids.iter().any(|id| id == DEEP_NESTING_ID);
        if !ids.iter().any(|id| id == SLOP_PROSE_ID) {
            self.slop_prose = None;
        }
        if !ids.iter().any(|id| id == DUPLICATION_ID) {
            self.duplication = None;
        }
        self.one_component = self.one_component && ids.iter().any(|id| id == ONE_COMPONENT_ID);
        self.effect_in_component = self.effect_in_component && ids.iter().any(|id| id == EFFECT_ID);
        self.prop_drilling = self.prop_drilling && ids.iter().any(|id| id == PROP_DRILLING_ID);
        self.store_passthrough =
            self.store_passthrough && ids.iter().any(|id| id == STORE_PASSTHROUGH_ID);
        self.unknown(ids)
    }

    /// Disable `ids`. Returns ids that don't exist.
    pub fn skip(&mut self, ids: &[String]) -> Vec<String> {
        for (i, rule) in self.rules.iter().enumerate() {
            if ids.iter().any(|id| id == rule.id()) {
                self.enabled[i] = false;
            }
        }
        if ids.iter().any(|id| id == FILE_SIZE_ID) {
            self.file_size_enabled = false;
        }
        if ids.iter().any(|id| id == DEEP_NESTING_ID) {
            self.nesting_enabled = false;
        }
        if ids.iter().any(|id| id == SLOP_PROSE_ID) {
            self.slop_prose = None;
        }
        if ids.iter().any(|id| id == DUPLICATION_ID) {
            self.duplication = None;
        }
        if ids.iter().any(|id| id == ONE_COMPONENT_ID) {
            self.one_component = false;
        }
        if ids.iter().any(|id| id == EFFECT_ID) {
            self.effect_in_component = false;
        }
        if ids.iter().any(|id| id == PROP_DRILLING_ID) {
            self.prop_drilling = false;
        }
        if ids.iter().any(|id| id == STORE_PASSTHROUGH_ID) {
            self.store_passthrough = false;
        }
        self.unknown(ids)
    }

    /// Ids the user named that don't correspond to any rule. Checks against *all*
    /// rule ids regardless of enabled state, so skipping a whole-run rule (which
    /// disables it) doesn't then report it as unknown.
    fn unknown(&self, ids: &[String]) -> Vec<String> {
        let mut known: Vec<&str> = self.rules.iter().map(|r| r.id()).collect();
        known.extend([
            FILE_SIZE_ID,
            DEEP_NESTING_ID,
            SLOP_PROSE_ID,
            DUPLICATION_ID,
            ONE_COMPONENT_ID,
            EFFECT_ID,
            PROP_DRILLING_ID,
            STORE_PASSTHROUGH_ID,
        ]);
        ids.iter()
            .filter(|id| !known.contains(&id.as_str()))
            .cloned()
            .collect()
    }

    /// Does any enabled rule look at this extension? Lets the caller skip reading
    /// files nothing will scan.
    pub fn handles_ext(&self, ext: &str) -> bool {
        if self.ext_skipped(ext) {
            return false;
        }
        self.rules
            .iter()
            .enumerate()
            .any(|(i, r)| self.enabled[i] && r.applies_to_ext(ext))
            || (self.file_size_enabled && SIZE_EXTS.contains(&ext))
            || (self.nesting_enabled && NEST_EXTS.contains(&ext))
            || (self.slop_prose.is_some() && PROSE_EXTS.contains(&ext))
            || (self.react_enabled() && REACT_EXTS.contains(&ext))
    }

    fn react_enabled(&self) -> bool {
        self.one_component
            || self.effect_in_component
            || self.prop_drilling
            || self.store_passthrough
    }

    /// Scan one file's text, honouring every `straitjacket-allow[-file]` marker. `path`
    /// is the display path; `ext` is lowercased and dot-free.
    pub fn scan_text(&self, text: &str, path: &str, ext: &str) -> Vec<Finding> {
        self.scan_inner(text, path, ext, true)
    }

    /// The *candidate* findings for a file, ignoring every allow marker: what would be
    /// reported if the file carried no suppression at all. Diffing this against
    /// [`scan_text`](Self::scan_text) tells us exactly which would-be violations a marker
    /// suppressed, which is how the `unused-marker` check learns whether a marker did
    /// anything. Only worth computing for files that actually carry a marker.
    pub fn scan_text_candidates(&self, text: &str, path: &str, ext: &str) -> Vec<Finding> {
        self.scan_inner(text, path, ext, false)
    }

    /// Shared scan body. When `suppress` is true this is the normal scan; when false every
    /// marker is ignored so the raw candidate set falls out (the two are diffed to attribute
    /// suppression to individual markers). Visible behaviour is unchanged: the CLI always
    /// calls the `suppress = true` path.
    fn scan_inner(&self, text: &str, path: &str, ext: &str, suppress: bool) -> Vec<Finding> {
        let mut findings = Vec::new();
        if self.ext_skipped(ext) {
            return findings;
        }

        // Which line rules are live for this file (enabled + applicable extension).
        let applies: Vec<bool> = self
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| self.enabled[i] && r.applies_to_ext(ext))
            .collect();
        let size_applies = self.file_size_enabled && SIZE_EXTS.contains(&ext);
        let nest_applies = self.nesting_enabled && NEST_EXTS.contains(&ext);
        let prose_applies = self.slop_prose.is_some() && PROSE_EXTS.contains(&ext);
        let react_applies = self.react_enabled() && REACT_EXTS.contains(&ext);
        if applies.iter().all(|b| !b)
            && !size_applies
            && !nest_applies
            && !prose_applies
            && !react_applies
        {
            return findings;
        }

        // Whole-file `straitjacket-allow-file[:rule]` directives. Only pay for the
        // line-by-line scan when the marker is actually present somewhere. In the
        // candidate (`!suppress`) pass we deliberately keep this empty so nothing is
        // suppressed.
        let file_allow = if suppress && text.contains(ALLOW_FILE) {
            FileAllow::scan(text)
        } else {
            FileAllow::default()
        };

        let mut total_lines = 0;
        for (idx, line) in text.lines().enumerate() {
            total_lines = idx + 1;
            let ctx = LineCtx {
                line,
                lineno: idx + 1,
                path,
                applies: &applies,
                allow: if suppress {
                    line_scope(line)
                } else {
                    Scope::None
                },
                file_allow: &file_allow,
            };

            // Regex-backed rules: the RegexSet says which could match this line;
            // non-regex rules (emoji) always run. Both go through `consider`.
            for set_i in self.set.matches(line).into_iter() {
                self.consider(self.set_to_rule[set_i], &ctx, &mut findings);
            }
            for &ri in &self.nonregex {
                self.consider(ri, &ctx, &mut findings);
            }
        }

        // Whole-file rule: line count over budget.
        if size_applies && !file_allow.covers(FILE_SIZE_ID) && total_lines > self.max_lines {
            findings.push(Finding {
                rule: FILE_SIZE_ID.to_string(),
                path: path.to_string(),
                line: self.max_lines + 1,
                col: 1,
                matched: format!("{total_lines} lines"),
                message: format!(
                    "file has {total_lines} lines, over the {}-line limit — sprawling single files are a common LLM tell; split it up.",
                    self.max_lines
                ),
                severity: Severity::Error,
            });
        }

        // Whole-file nesting rule: indentation depth over budget. Findings are
        // per line, so a line-scoped `straitjacket-allow` on the offending line
        // silences it (same idiom as the React rules below).
        if nest_applies && !file_allow.covers(DEEP_NESTING_ID) {
            for f in nesting::scan(text, path, self.max_nesting) {
                let allowed = suppress
                    && text
                        .lines()
                        .nth(f.line - 1)
                        .is_some_and(|l| scope_covers(&line_scope(l), &f.rule));
                if !allowed {
                    findings.push(f);
                }
            }
        }

        // Whole-text prose analyzer.
        if prose_applies && !file_allow.covers(SLOP_PROSE_ID) {
            if let Some(sp) = &self.slop_prose {
                findings.extend(sp.scan(text, path, suppress));
            }
        }

        // React AST rules (.tsx/.jsx). Honour both file- and line-scoped allows.
        if react_applies {
            let one_component = self.one_component && !file_allow.covers(ONE_COMPONENT_ID);
            let effect = self.effect_in_component && !file_allow.covers(EFFECT_ID);
            let drilling = self.prop_drilling && !file_allow.covers(PROP_DRILLING_ID);
            let store = self.store_passthrough && !file_allow.covers(STORE_PASSTHROUGH_ID);
            if one_component || effect || drilling || store {
                let idx = self.component_index.as_ref();
                for f in react::analyze(text, path, one_component, effect, drilling, store, idx) {
                    let allowed = suppress
                        && text
                            .lines()
                            .nth(f.line - 1)
                            .is_some_and(|l| scope_covers(&line_scope(l), &f.rule));
                    if !allowed {
                        findings.push(f);
                    }
                }
            }
        }

        findings
    }

    /// Run one line rule (by index) against a line, honouring extension
    /// applicability and the line/file allow markers, appending any findings.
    fn consider(&self, ri: usize, ctx: &LineCtx, out: &mut Vec<Finding>) {
        let id = self.rules[ri].id();
        if !ctx.applies[ri] || ctx.file_allow.covers(id) || scope_covers(&ctx.allow, id) {
            return;
        }
        let rule = &self.rules[ri];
        let mut hits: Vec<LineHit> = Vec::new();
        rule.scan_line(ctx.line, &mut hits);
        for hit in hits {
            out.push(Finding {
                rule: rule.id().to_string(),
                path: ctx.path.to_string(),
                line: ctx.lineno,
                col: hit.col,
                matched: hit.matched,
                message: rule.message().to_string(),
                severity: Severity::Error,
            });
        }
    }
}

/// Per-line context shared by every line rule via [`Engine::consider`].
struct LineCtx<'a> {
    line: &'a str,
    lineno: usize,
    path: &'a str,
    applies: &'a [bool],
    allow: Scope,
    file_allow: &'a FileAllow,
}

/// What a `straitjacket-allow[-file]` marker covers.
enum Scope {
    None,
    /// Bare marker — every rule.
    All,
    /// `…:<id>` — only this rule.
    Only(String),
}

fn scope_covers(scope: &Scope, id: &str) -> bool {
    match scope {
        Scope::None => false,
        Scope::All => true,
        Scope::Only(only) => only == id,
    }
}

/// Parse the `:<id>` suffix (if any) that follows a marker keyword.
fn scope_from_rest(rest: &str) -> Scope {
    if let Some(after) = rest.strip_prefix(':') {
        let id: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if id.is_empty() {
            Scope::All
        } else {
            Scope::Only(id)
        }
    } else {
        Scope::All
    }
}

/// The line-scoped `straitjacket-allow[:rule]` directive on this line, if any. The
/// longer `straitjacket-allow-file` token is deliberately not treated as a line
/// marker (it's whole-file), so occurrences of it are skipped.
fn line_scope(line: &str) -> Scope {
    let mut start = 0;
    while let Some(rel) = line[start..].find(ALLOW) {
        let pos = start + rel;
        let rest = &line[pos + ALLOW.len()..];
        if rest.starts_with("-file") {
            start = pos + ALLOW.len();
            continue;
        }
        return scope_from_rest(rest);
    }
    Scope::None
}

/// The whole-file `straitjacket-allow-file[:rule]` directive on this line, if any.
fn file_scope(line: &str) -> Scope {
    match line.find(ALLOW_FILE) {
        Some(pos) => scope_from_rest(&line[pos + ALLOW_FILE.len()..]),
        None => Scope::None,
    }
}

/// Aggregated whole-file exemptions: which rules (or all of them) a file opts out of
/// via `straitjacket-allow-file` markers anywhere in its text.
#[derive(Default)]
struct FileAllow {
    all: bool,
    rules: Vec<String>,
}

impl FileAllow {
    fn scan(text: &str) -> Self {
        let mut fa = FileAllow::default();
        for line in text.lines() {
            match file_scope(line) {
                Scope::None => {}
                Scope::All => fa.all = true,
                Scope::Only(id) => fa.rules.push(id),
            }
        }
        fa
    }

    fn covers(&self, id: &str) -> bool {
        self.all || self.rules.iter().any(|r| r == id)
    }
}

/// Whether a finding for rule `id` at 1-based `line` in a file whose contents are `text`
/// is suppressed by a `straitjacket-allow`/`-file` marker. The per-file rules apply this
/// as they scan (via `LineCtx`); the cross-file duplication pass runs separately, so it
/// calls this afterwards to honour the same markers.
pub fn is_suppressed(text: &str, line: usize, id: &str) -> bool {
    if text.contains(ALLOW_FILE) && FileAllow::scan(text).covers(id) {
        return true;
    }
    if line > 0 {
        if let Some(l) = text.lines().nth(line - 1) {
            if scope_covers(&line_scope(l), id) {
                return true;
            }
        }
    }
    false
}

/// One suppression marker as it appears in a file: the line it sits on, whether it is a
/// whole-file (`straitjacket-allow-file`) or a line-scoped (`straitjacket-allow`) marker,
/// and the rule it names — `None` for a bare marker that covers every rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// 1-based line the marker sits on.
    pub line: usize,
    /// `true` for `straitjacket-allow-file`, `false` for a line-scoped `straitjacket-allow`.
    pub file_level: bool,
    /// The named rule, or `None` for a bare (all-rules) marker.
    pub rule: Option<String>,
}

/// A would-be violation that a marker suppressed: the rule it belonged to and the 1-based
/// line it sat on. This is what a marker has to have covered at least once to count as used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppressed {
    pub rule: String,
    pub line: usize,
}

/// Every *deliberate* `straitjacket-allow[-file]` directive in `text`, in file order.
///
/// This is stricter than the suppression gate on purpose. Suppression treats any occurrence
/// of the substring as a marker — harmless, because a line with no finding suppresses nothing
/// anyway. The unused-marker check can't be that loose: straitjacket's own sources are
/// saturated with the word (doc comments, the `ALLOW`/`ALLOW_FILE` constants, the
/// `straitjacket-allow[:rule]` documentation notation, prose in the CHANGELOG), and flagging
/// every mention would bury the signal. So a directive is only collected when it is *bounded*
/// like a real one: preceded by start-of-line, whitespace, or comment punctuation (never a
/// word char, backtick, or quote) and immediately followed by `:`, whitespace, or
/// end-of-line (never `[`, a backtick, or a quote). Suppression semantics are untouched — this
/// function feeds only the unused-marker reconciliation.
pub fn collect_markers(text: &str) -> Vec<Marker> {
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        collect_line_markers(line, idx + 1, &mut out);
    }
    out
}

/// Whether the character just before a candidate marker is a legitimate boundary (so the token
/// isn't part of a larger word, a quoted string, or backticked prose).
fn valid_marker_prefix(c: Option<char>) -> bool {
    match c {
        None => true,
        Some(c) => !c.is_alphanumeric() && !matches!(c, '`' | '"' | '\'' | '[' | '_'),
    }
}

/// Whether the character just after the marker keyword makes it a real directive: a scope
/// separator (`:`), the bracket form's `[` (`straitjacket-allow-file[:rule]`, a common mistake
/// that resolves to *all rules* — worth catching), whitespace, or the end of the line. A
/// backtick, quote, letter, or other punctuation means it's a mention, not a directive.
fn valid_marker_suffix(c: Option<char>) -> bool {
    matches!(c, None | Some(':') | Some('[')) || c.is_some_and(char::is_whitespace)
}

fn collect_line_markers(line: &str, lineno: usize, out: &mut Vec<Marker>) {
    let mut search = 0;
    while let Some(rel) = line[search..].find(ALLOW) {
        let pos = search + rel;
        search = pos + ALLOW.len();
        let file_level = line[pos..].starts_with(ALLOW_FILE);
        let kw_len = if file_level {
            ALLOW_FILE.len()
        } else {
            ALLOW.len()
        };
        let before = line[..pos].chars().next_back();
        let rest = &line[pos + kw_len..];
        let after = rest.chars().next();
        if !valid_marker_prefix(before) || !valid_marker_suffix(after) {
            continue;
        }
        // An odd count of `"` or `` ` `` before the marker means it sits inside an open
        // string or backtick span — a test fixture or an example embedded in a code string,
        // not a directive (the closing quote may even land on a later continuation line).
        // Skip it, so the check doesn't flag marker syntax that appears as data.
        let head = &line[..pos];
        if head.matches('"').count() % 2 == 1 || head.matches('`').count() % 2 == 1 {
            continue;
        }
        let rule = match scope_from_rest(rest) {
            Scope::All => None,
            Scope::Only(id) => Some(id),
            Scope::None => continue,
        };
        out.push(Marker {
            line: lineno,
            file_level,
            rule,
        });
    }
}

/// Would marker `m` have suppressed the would-be violation `s`? Mirrors the suppression
/// gate: a file-level marker covers a matching rule anywhere; a line-level marker covers a
/// matching rule only on its own line. The `slop-prose` clause matches that rule's divergent
/// raw-substring path (any marker sharing the artifact's line silences it, scope-agnostically),
/// so a scope-mismatched marker that really did silence a slop artifact still reads as used.
fn marker_covers(m: &Marker, s: &Suppressed) -> bool {
    let rule_match = m.rule.as_deref().is_none_or(|id| id == s.rule);
    if m.file_level {
        rule_match || (m.line == s.line && s.rule == SLOP_PROSE_ID)
    } else {
        m.line == s.line && (rule_match || s.rule == SLOP_PROSE_ID)
    }
}

impl Engine {
    /// Is `id` an enabled rule that looks at files with extension `ext`? Covers every rule
    /// family, including the whole-file and cross-file ones. `duplication` is cross-file, so
    /// it is "active" for any file the scan reads (json is already excluded upstream).
    fn rule_active_for_ext(&self, id: &str, ext: &str) -> bool {
        match id {
            FILE_SIZE_ID => self.file_size_enabled && SIZE_EXTS.contains(&ext),
            DEEP_NESTING_ID => self.nesting_enabled && NEST_EXTS.contains(&ext),
            SLOP_PROSE_ID => self.slop_prose.is_some() && PROSE_EXTS.contains(&ext),
            DUPLICATION_ID => self.duplication.is_some(),
            ONE_COMPONENT_ID => self.one_component && REACT_EXTS.contains(&ext),
            EFFECT_ID => self.effect_in_component && REACT_EXTS.contains(&ext),
            PROP_DRILLING_ID => self.prop_drilling && REACT_EXTS.contains(&ext),
            STORE_PASSTHROUGH_ID => self.store_passthrough && REACT_EXTS.contains(&ext),
            _ => self
                .rules
                .iter()
                .enumerate()
                .any(|(i, r)| r.id() == id && self.enabled[i] && r.applies_to_ext(ext)),
        }
    }

    /// Does any enabled rule at all look at `ext`? Used to decide whether a bare (all-rules)
    /// marker could possibly do anything on this file.
    fn any_rule_active_for_ext(&self, ext: &str) -> bool {
        self.duplication.is_some()
            || (self.file_size_enabled && SIZE_EXTS.contains(&ext))
            || (self.nesting_enabled && NEST_EXTS.contains(&ext))
            || (self.slop_prose.is_some() && PROSE_EXTS.contains(&ext))
            || (self.react_enabled() && REACT_EXTS.contains(&ext))
            || self
                .rules
                .iter()
                .enumerate()
                .any(|(i, r)| self.enabled[i] && r.applies_to_ext(ext))
    }

    /// Whether a marker could conceivably suppress anything in a file of this extension.
    /// A marker whose rule can't run here (e.g. a `color` marker in a Markdown file, where
    /// `color` never applies) is *inert*, not unused: it is skipped, never flagged. This is
    /// what keeps documentation that shows example markers from tripping the check.
    fn marker_is_eligible(&self, m: &Marker, ext: &str) -> bool {
        match &m.rule {
            Some(id) => self.rule_active_for_ext(id, ext),
            None => self.any_rule_active_for_ext(ext),
        }
    }

    /// Turn the markers in one scanned file into `unused-marker` findings: one per marker
    /// that suppressed nothing. `suppressed` is every would-be violation a marker dropped in
    /// this file (per-file rules via the [`scan_text`](Self::scan_text) /
    /// [`scan_text_candidates`](Self::scan_text_candidates) diff, plus any `duplication`
    /// clones dropped in the cross-file pass). `dup_partner`, when set, is the
    /// alphabetically-first file of a clone pair whose *second* file is this one — used to
    /// explain that a `duplication` marker here is dead by construction.
    pub fn unused_marker_findings(
        &self,
        path: &str,
        ext: &str,
        markers: &[Marker],
        suppressed: &[Suppressed],
        dup_partner: Option<&str>,
    ) -> Vec<Finding> {
        let mut out = Vec::new();
        for m in markers {
            if !self.marker_is_eligible(m, ext) {
                continue;
            }
            if suppressed.iter().any(|s| marker_covers(m, s)) {
                continue;
            }
            out.push(self.unused_finding(path, m, dup_partner));
        }
        out
    }

    /// Build the `unused-marker` finding for a single dead marker.
    fn unused_finding(&self, path: &str, m: &Marker, dup_partner: Option<&str>) -> Finding {
        let keyword = if m.file_level { ALLOW_FILE } else { ALLOW };
        let matched = match &m.rule {
            Some(id) => format!("{keyword}:{id}"),
            None => keyword.to_string(),
        };
        // A wrong-side duplication marker: it names duplication (or is bare, which covers it)
        // and lives on the *second* file of a clone pair, where the detector never reads it.
        let targets_dup = m.rule.as_deref().is_none_or(|id| id == DUPLICATION_ID);
        let message = match dup_partner {
            Some(a) if targets_dup => format!(
                "unused duplication marker — straitjacket reads the marker only on the \
                 alphabetically-first file of a clone pair ({a}); this marker is on the second \
                 file. Move it there or remove it."
            ),
            _ => {
                let what = match &m.rule {
                    Some(id) => id.as_str(),
                    None => "all rules",
                };
                format!(
                    "unused suppression marker ({what}) — it suppressed no findings; remove it."
                )
            }
        };
        Finding {
            rule: UNUSED_MARKER_ID.to_string(),
            path: path.to_string(),
            line: m.line,
            col: 1,
            matched,
            message,
            severity: Severity::Error,
        }
    }
}

/// The would-be violations a file's markers suppressed, found by diffing the candidate
/// findings (markers ignored) against the visible findings (markers honoured): anything in
/// the candidate set that isn't visible was suppressed. Compared on `(rule, line, col)` so
/// two findings from the same rule on the same line aren't conflated.
pub fn suppressed_between(candidates: &[Finding], visible: &[Finding]) -> Vec<Suppressed> {
    candidates
        .iter()
        .filter(|c| {
            !visible
                .iter()
                .any(|v| v.rule == c.rule && v.line == c.line && v.col == c.col)
        })
        .map(|c| Suppressed {
            rule: c.rule.clone(),
            line: c.line,
        })
        .collect()
}

#[cfg(test)]
mod suppress_tests {
    use super::is_suppressed;

    #[test]
    fn file_marker_suppresses_the_named_rule() {
        let text = "// straitjacket-allow-file:duplication reason\nfoo\nbar\n";
        assert!(is_suppressed(text, 2, "duplication"));
        // ...but only that rule.
        assert!(!is_suppressed(text, 2, "color"));
    }

    #[test]
    fn bare_file_marker_suppresses_any_rule() {
        let text = "// straitjacket-allow-file generated\nfoo\n";
        assert!(is_suppressed(text, 2, "duplication"));
        assert!(is_suppressed(text, 2, "color"));
    }

    #[test]
    fn line_marker_suppresses_its_own_line() {
        let text = "foo\nbar // straitjacket-allow:duplication\nbaz\n";
        assert!(is_suppressed(text, 2, "duplication"));
        assert!(!is_suppressed(text, 1, "duplication"));
        assert!(!is_suppressed(text, 3, "duplication"));
    }

    #[test]
    fn no_marker_is_not_suppressed() {
        assert!(!is_suppressed("just some code\n", 1, "duplication"));
    }
}

#[cfg(test)]
mod marker_tests {
    use super::{collect_markers, suppressed_between, Marker, Suppressed};
    use crate::finding::{Finding, Severity};

    fn f(rule: &str, line: usize, col: usize) -> Finding {
        Finding {
            rule: rule.to_string(),
            path: "t".to_string(),
            line,
            col,
            matched: String::new(),
            message: String::new(),
            severity: Severity::Error,
        }
    }

    #[test]
    fn collects_a_scoped_file_directive() {
        let m = collect_markers("// straitjacket-allow-file:duplication generated\ncode\n");
        assert_eq!(
            m,
            vec![Marker {
                line: 1,
                file_level: true,
                rule: Some("duplication".to_string()),
            }]
        );
    }

    #[test]
    fn collects_a_scoped_line_directive() {
        let m = collect_markers("code // straitjacket-allow:color\n");
        assert_eq!(
            m,
            vec![Marker {
                line: 1,
                file_level: false,
                rule: Some("color".to_string()),
            }]
        );
    }

    #[test]
    fn bracket_form_is_a_bare_all_marker() {
        // `[:rule]` doesn't parse as a scope (it starts with `[`, not `:`), so this common
        // mistake resolves to *all rules* — and must still be recognized as a directive.
        let m = collect_markers("/* straitjacket-allow-file[:duplication] */\n");
        assert_eq!(
            m,
            vec![Marker {
                line: 1,
                file_level: true,
                rule: None,
            }]
        );
    }

    #[test]
    fn ignores_marker_syntax_inside_a_string() {
        // An odd number of quotes before the token means it's string data, not a directive —
        // the classic test-fixture shape, where a marker lives inside a source string.
        assert!(collect_markers("let s = \"// straitjacket-allow:color\";\n").is_empty());
        // A multi-line fixture string with the marker on the opening-quote line, too.
        assert!(collect_markers(
            "let src = \"// straitjacket-allow-file:color generated\\n\\\n  code\";\n"
        )
        .is_empty());
    }

    #[test]
    fn ignores_backticked_prose_and_bracket_notation() {
        // A backtick immediately around the token (inline code / prose) is not a directive.
        assert!(collect_markers("the `straitjacket-allow` escape hatch\n").is_empty());
        // Nor is the token buried in a longer word.
        assert!(collect_markers("xstraitjacket-allow:color\n").is_empty());
    }

    #[test]
    fn suppressed_between_diffs_on_position() {
        let candidates = vec![f("color", 2, 1), f("emoji", 3, 5)];
        let visible = vec![f("emoji", 3, 5)];
        assert_eq!(
            suppressed_between(&candidates, &visible),
            vec![Suppressed {
                rule: "color".to_string(),
                line: 2,
            }]
        );
    }
}
