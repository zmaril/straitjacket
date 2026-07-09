//! Per-language binding specs: a tree-sitter grammar plus one query written in
//! the fixed capture vocabulary documented in [`super`]. The generic engine in
//! [`super::analysis`] is the only consumer; adding a language means adding a
//! grammar dependency, a `queries/<lang>.scm`, and an entry in [`for_ext`].

use std::sync::OnceLock;

use tree_sitter::{Language, Query};

/// What a capture name in the vocabulary means to the engine. Parallel to the
/// query's capture table (`caps[capture_index]`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Cap {
    Ref,
    Ignore,
    Def,
    DefBare,
    DefParam,
    DefPattern,
    DefHoist,
    PatternScope,
    Anchor,
    Assign,
    AssignUpdate,
    AssignMulti,
    ScopeBlock,
    ScopeFunction,
    ScopeOpaque,
    Loop,
    Branch,
    Escape,
    StringInterp,
}

fn cap_for(name: &str) -> Cap {
    match name {
        "ref" => Cap::Ref,
        "ignore" => Cap::Ignore,
        "def" => Cap::Def,
        "def.bare" => Cap::DefBare,
        "def.param" => Cap::DefParam,
        "def.pattern" => Cap::DefPattern,
        "def.pattern.scope" => Cap::PatternScope,
        "def.hoist" => Cap::DefHoist,
        "anchor" => Cap::Anchor,
        "assign" => Cap::Assign,
        "assign.update" => Cap::AssignUpdate,
        "assign.multi" => Cap::AssignMulti,
        "scope" => Cap::ScopeBlock,
        "scope.function" => Cap::ScopeFunction,
        "scope.opaque" => Cap::ScopeOpaque,
        "loop" => Cap::Loop,
        "branch" => Cap::Branch,
        "escape" => Cap::Escape,
        "string.interp" => Cap::StringInterp,
        other => panic!("dataflow query uses unknown capture @{other}"),
    }
}

/// A language's binding spec: grammar, compiled query, and the handful of
/// engine knobs that are per-language facts rather than query-expressible.
pub(super) struct LanguageSpec {
    pub language: Language,
    pub query: Query,
    /// Capture index → vocabulary kind.
    pub caps: Vec<Cap>,
    /// Node kinds that bind a name when found inside a `@def.pattern` /
    /// `@def.param` / `@assign.multi` subtree.
    pub binding_kinds: &'static [&'static str],
    /// Child field names skipped during that extraction (type annotations,
    /// default values — their identifiers are reads, not bindings).
    pub skip_fields: &'static [&'static str],
    /// Python-style scoping: no block scopes, assignment anywhere in a
    /// function makes the name local to it, and all bindings of a name in one
    /// scope are the same variable (no shadowing).
    pub merged_vars: bool,
}

fn build(
    language: Language,
    query_src: &str,
    binding_kinds: &'static [&'static str],
    skip_fields: &'static [&'static str],
    merged_vars: bool,
) -> LanguageSpec {
    let query = Query::new(&language, query_src).expect("dataflow query compiles");
    let caps = query.capture_names().iter().map(|n| cap_for(n)).collect();
    LanguageSpec {
        language,
        query,
        caps,
        binding_kinds,
        skip_fields,
        merged_vars,
    }
}

const RUST_QUERY: &str = include_str!("queries/rust.scm");
const TS_QUERY: &str = include_str!("queries/typescript.scm");
const PY_QUERY: &str = include_str!("queries/python.scm");

/// The spec for a file extension (lowercased, dot-free), if the language is
/// supported. Specs are built once per process on first use.
pub(super) fn for_ext(ext: &str) -> Option<&'static LanguageSpec> {
    static RUST: OnceLock<LanguageSpec> = OnceLock::new();
    static TS: OnceLock<LanguageSpec> = OnceLock::new();
    static TSX: OnceLock<LanguageSpec> = OnceLock::new();
    static PY: OnceLock<LanguageSpec> = OnceLock::new();

    const TS_BINDING_KINDS: &[&str] = &["identifier", "shorthand_property_identifier_pattern"];
    const TS_SKIP_FIELDS: &[&str] = &["type", "right", "value", "key", "return_type"];

    match ext {
        "rs" => Some(RUST.get_or_init(|| {
            build(
                tree_sitter_rust::LANGUAGE.into(),
                RUST_QUERY,
                &["identifier", "shorthand_field_identifier"],
                // `condition` skips match-arm guards: their identifiers are
                // reads, not bindings.
                &["type", "condition"],
                false,
            )
        })),
        "ts" | "js" | "mjs" | "cjs" => Some(TS.get_or_init(|| {
            build(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                TS_QUERY,
                TS_BINDING_KINDS,
                TS_SKIP_FIELDS,
                false,
            )
        })),
        "tsx" | "jsx" => Some(TSX.get_or_init(|| {
            build(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                TS_QUERY,
                TS_BINDING_KINDS,
                TS_SKIP_FIELDS,
                false,
            )
        })),
        "py" => Some(PY.get_or_init(|| {
            build(
                tree_sitter_python::LANGUAGE.into(),
                PY_QUERY,
                &["identifier"],
                &["type", "value", "return_type"],
                true,
            )
        })),
        _ => None,
    }
}
