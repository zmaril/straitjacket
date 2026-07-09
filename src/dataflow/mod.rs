//! Experimental language-generic dataflow analysis over tree-sitter, behind the
//! opt-in `--dataflow` flag (config key `dataflow`). Off by default.
//!
//! The architecture is "per-language binding spec + one generic engine": each
//! supported language contributes a single tree-sitter query file
//! (`src/dataflow/queries/*.scm`) whose capture names come from a fixed
//! vocabulary, and one generic engine ([`analysis`]) parses the file, runs the
//! query, builds a scope tree, resolves names lexically, and computes a simple
//! function-local def-use analysis. The only check shipped by the prototype is
//! **`unused-assignment`**: a value assigned to a local variable that is never
//! read before the variable is reassigned or goes out of scope.
//!
//! # Capture vocabulary
//!
//! | capture | meaning |
//! |---|---|
//! | `@ref` | an identifier that reads a name (captured broadly; anything not re-classified below stays a read) |
//! | `@ignore` | an identifier that is *not* a name use (attribute/field names, macro names); on a non-identifier node, every `@ref` inside its range is dropped |
//! | `@def` | a declaration binding a name to a value — the flaggable kind (`let x = e`, `const x = e`) |
//! | `@def.bare` | a declaration without a value (`let x;`) — resolution only |
//! | `@def.param` | a parameter list or parameter binding — bound names extracted, never flagged |
//! | `@def.pattern` | a destructuring/match pattern (or import) — bound names extracted, never flagged |
//! | `@def.pattern.scope` | paired with `@def.*` in the same query match: the node the bindings are scoped to (match arm body, if-let consequence) |
//! | `@def.hoist` | a binding hoisted into the scope *enclosing* its nearest scope (function/enum names), visible from the top |
//! | `@anchor` | paired with `@def`/`@assign*` in the same match: the binding becomes visible / the write takes effect at the *end* of this node, so `let x = x + 1` reads the outer `x` |
//! | `@assign` | a plain reassignment of a single name — the flaggable write kind |
//! | `@assign.update` | a read-then-write (`x += 1`, `x++`) — records a read *and* a write; never flagged itself |
//! | `@assign.multi` | a write extracted from a pattern (destructuring assignment, loop targets, `with ... as`) — never flagged |
//! | `@scope` | a block scope |
//! | `@scope.function` | a function scope — the unit of analysis; also closures, lambdas, and Python comprehensions (their bodies run "elsewhere") |
//! | `@scope.opaque` | a scope whose bindings are externally visible (class bodies) — nothing inside is ever flagged |
//! | `@loop` | a loop node — a write in a loop with any read of the same variable in that loop is exempt (back edges) |
//! | `@branch` | a conditional construct (if/match/switch/try/ternary/short-circuit) — a "reassigned before read" pair only flags when the killing write is not more conditional than the killed one |
//! | `@escape` | a name declared `global`/`nonlocal` — never flagged in that function |
//! | `@string.interp` | a string literal scanned for `{name}` interpolations that count as reads (Rust format!-style macros) |
//!
//! # What this analysis deliberately does NOT do
//!
//! The check is function-local and syntactic. There is **no aliasing** (a write
//! through `*p` or `obj.field` never counts as a write to a tracked variable —
//! the base name only counts as a read), **no cross-function flow**, **no type
//! information**, and **no real control-flow graph** (conditionality is
//! approximated by nesting of `@branch` nodes; loops by a blanket "any read of
//! the variable inside the loop exempts writes in that loop" rule). Names that
//! escape into a nested function/closure are never flagged, reads the query
//! cannot classify stay reads, and a name that cannot be resolved anywhere masks
//! every same-named variable in the file. Python `match` patterns and `del` are
//! not modelled (their identifiers degrade to reads). Files that fail to parse
//! cleanly are skipped entirely. All of this errs toward **not flagging**:
//! false positives are the failure mode that matters.

mod analysis;
mod spec;

use crate::finding::Finding;

/// Rule id of the one dataflow check shipped by the prototype.
pub const UNUSED_ASSIGNMENT_ID: &str = "unused-assignment";

/// Extensions the dataflow analysis knows how to parse.
pub const DATAFLOW_EXTS: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py"];

/// Run the dataflow analysis on one file's text. `ext` is lowercased and
/// dot-free, matching the engine's dispatch convention. Unknown extensions and
/// files with parse errors produce no findings.
pub fn analyze(text: &str, path: &str, ext: &str) -> Vec<Finding> {
    match spec::for_ext(ext) {
        Some(spec) => analysis::run(spec, text, path),
        None => Vec::new(),
    }
}
