# Language-generic dataflow via tree-sitter binding specs

Status: prototype implemented behind the opt-in `--dataflow` flag (config key
`dataflow`, off by default). One check ships: `unused-assignment`. Design record
for the branch that added `src/dataflow/`.

## Motivation

Straitjacket's only real parser today is oxc, and oxc is JS/TS-only. Every other
language gets the non-parsing tiers: regex over lines, codepoint scans,
indentation counting, cpd-finder's tokenizer. That ceiling matters — the checks
that catch what LLMs actually get wrong in *logic* (a value computed and never
used, a variable clobbered before it is read) need bindings and def-use, and we
can offer those to exactly two extensions. The fleet is mostly Rust and Python
by line count; those files get no semantic checks at all.

The question this note answers: what is the cheapest honest way to bring
dataflow-grade checks to the languages oxc will never cover, without breaking
the constraints that define the tool — a single static Rust binary, no user-side
toolchain, deterministic output, documented limits, and a strong bias toward
silence over false alarms.

## What stays on oxc

Porting the existing React rules off oxc was considered and rejected. The six
oxc-backed analyzers grade like this (full detail in the per-check audit that
preceded this branch):

| check | semantic depth actually used | verdict |
|---|---|---|
| `one-component` | raw AST + spans (PascalCase heuristic, JSX visitor) | portable in mechanism, but React/JSX-specific — porting buys nothing |
| `effect-in-component` | raw AST + span containment | same |
| `extract_edges` (`--prop-chains`) | raw AST, deliberately name-based | same |
| `prop-drilling` | oxc semantic model: scopes, bindings, resolved references | keep on oxc |
| `store-passthrough` | same resolved-reference walk, different seed bindings | keep on oxc |
| `ComponentIndex` (callback-slot filter) | syntactic TS type-annotation reading | keep on oxc — TS-specific by nature |

Three checks need only a parse tree, so they *could* move to tree-sitter — but
they only make sense for JSX, so a generic parser gains no reach. The two
forwarding rules genuinely depend on oxc's scope/binding/resolved-reference
machinery, which a bare tree-sitter CST does not provide. And anything
type-adjacent (the `ComponentIndex` annotation reading, any future TS-aware
rule) is simply better on oxc. Conclusion: **oxc stays for JS/TS; the generic
engine exists for reach into Rust, Python, Go, and whatever the fleet grows
next** — not to delete a parser that is doing its job.

## Design space considered

Surveyed mid-2026; the load-bearing facts, with sources:

- **stack-graphs / tree-sitter-stack-graphs** (GitHub's code-nav stack).
  Archived by GitHub on Sep 9, 2025, read-only, "fork it if you wish to
  continue" ([repo](https://github.com/github/stack-graphs)); its TSG DSL crate
  last released Dec 2024. Authoring cost was the worst ever demonstrated for
  this shape: the TypeScript rules alone are a ~6,300-line `.tsg` file, and only
  four languages ever shipped. Semantic ceiling is name resolution — def-to-refs
  with no assignment ordering, no kills, no flow — so `unused-assignment` is
  outside the *model*, not just unimplemented. Rejected: a dead fork that costs
  the most per language and delivers less than the target checks need.
- **Semgrep's architecture** (per-language CST-to-generic-AST translators, one
  IL, language-agnostic dataflow;
  [overview](https://docs.semgrep.dev/writing-rules/data-flow/data-flow-overview)).
  The right north star: one generic core, per-language lowering. But the engine
  is LGPL-2.1 OCaml — nothing to link from Rust — cross-file taint is the
  proprietary Pro engine, and the community fork
  ([Opengrep](https://www.opengrep.dev/), active through 2026) would have to be
  shelled out to, breaking the single-binary ethos. Copying the design means a
  hand-written translator per language at person-weeks each — over-budget as a
  first step, though the internal interface below is deliberately the front half
  of it.
- **CodeQL, Glean, SCIP/LSIF, Joern**: each fails an absolute constraint.
  CodeQL's toolchain is proprietary (free only for OSI-licensed code) and a
  batch database pipeline; Glean is a Haskell service fed by external indexers;
  SCIP indexers require the user to have each language's own toolchain — the
  failure mode straitjacket exists to avoid — and carry no statement-level flow;
  Joern is a Scala/JVM analysis platform, unembeddable in a small static binary.
- **Chosen: a hand-rolled binding spec per language over tree-sitter queries,
  one generic engine in-crate.** tree-sitter itself is healthy (0.26.x, MIT,
  first-class Rust bindings) and grammars compile into the static binary.
  tree-sitter's own `locals.scm` convention proves the shape works at tens of
  lines per language, though it stops at innermost-scope name coloring; the
  survey found no off-the-shelf generic def-use engine over tree-sitter — the
  engine is genuinely the novel part. One lesson vendored in from the ecosystem:
  nvim-treesitter, the largest curated query collection, was archived in
  April 2026, so straitjacket owns its query files outright rather than
  depending on any external collection.

## The prototype

The architecture is "per-language binding spec + one generic engine":

- **A fixed capture vocabulary** (documented in `src/dataflow/mod.rs`): `@def`,
  `@def.param`, `@def.pattern`, `@def.hoist`, `@assign`, `@assign.update`,
  `@ref`, `@ignore`, `@scope`, `@scope.function`, `@scope.opaque`, `@loop`,
  `@branch`, `@escape`, `@string.interp`, plus anchors for
  binding-visible-at-end-of-node semantics (`let x = x + 1` reads the outer
  `x`).
- **Per-language specs**: one `.scm` query file per language
  (`src/dataflow/queries/{rust,typescript,python}.scm`, 68–79 lines each)
  plus a small quirks table in `src/dataflow/spec.rs` (which grammar, which
  extraction skip-fields, hoisting behavior). Adding a language touches only
  these.
- **One generic engine** (`src/dataflow/analysis.rs`): parse, run the query,
  build the scope tree, resolve names lexically, then a function-local def-use
  pass. The one check, `unused-assignment` (Warning), flags a value assigned to
  a local variable that is never read before reassignment or scope end.
- **Dispatch** follows the house pattern: extension-gated
  (`rs/ts/tsx/js/jsx/mjs/cjs/py`), wired through `Engine::scan_text` like the
  other whole-file analyzers, suppressible with the usual allow markers, off
  unless `--dataflow` is passed.

The safety property the whole design leans on: **every ambiguity degrades to a
read.** An identifier the query does not classify stays a read; a name that
resolves nowhere masks every same-named variable in the file; closure-touched
variables, `@assign.update`, and destructuring writes are never flagged. A gap
in a `.scm` file can therefore *mask* a real finding but can never *invent* a
false one — the failure mode that matters for a tool that hard-fails CI.

### Measured results

Method: run `--no-config --only unused-assignment --dataflow --no-fail` over the
fleet (powdermonkey, entl, disponent, straitjacket; 752 files) and inspect every
finding by hand. The first working build produced 4 findings — all four false
positives, in two classes: three where a whole `if`/`try` construct was one
branch region, so writes in opposite arms killed each other; one where a Rust
match-arm guard identifier was extracted as a pattern binding instead of a read.
Both classes were fixed (arm-granular `@branch` captures in all three query
files; `condition` added to the Rust extraction skip-fields) and pinned with
regression tests. The committed build reports **zero findings on the fleet** —
expected for a precision-first rule over compiler/clippy/biome-clean code.

Zero findings could also mean a broken analyzer, so recall was probed by
seeding four known dead stores into copies of real fleet files (TS, TSX, Rust,
Python): 4/4 detected, no extra findings. The behavioural suite
(`tests/dataflow.rs`) pins 8 true-positive patterns and roughly 30 tricky
negatives (shadowing, if-let/match bindings, augmented assignment,
destructuring, closures, loop-carried values, conditional overwrites,
format-string/f-string/template reads).

Cost: the full powdermonkey scan goes from 0.15s to 0.47s with `--dataflow`
(415 files, release build) — acceptable for an opt-in pass.

## Honest limits

The check is function-local and syntactic, and the misses are documented in
`src/dataflow/mod.rs`:

- **No CFG.** Conditionality is approximated by `@branch` nesting; loops get a
  blanket back-edge exemption. An early `return`, `?`, or `break` between two
  writes is not modelled — which can only suppress a kill-flag, never create
  one (false-negative-only by construction).
- **No aliasing, no cross-function flow, no types.** A write through `*p` or
  `obj.field` never counts as a write to a tracked variable.
- Python `match` patterns and `del` are unmodelled (identifiers degrade to
  reads); exotic Rust const-patterns in match arms extract as bindings.
- Rust `format!`-style `{name}` interpolation is recovered by scanning string
  literals inside macro token trees, so a brace-name in any *other* macro string
  also counts as a read (conservative).
- Files that do not parse cleanly under the grammar are skipped silently —
  notably `.js` files containing JSX, which the TS grammar rejects.

## Follow-ups

- More languages: Go first (`:=` vs `=`, receivers, named returns are the known
  quirks to encode). Each language is a `.scm` file + a spec entry + fixtures.
- A second check on the same engine: use-before-def, hoisting-aware via the
  quirks table.
- If taint ever becomes a hard requirement, the upgrade path is a per-language
  CFG-lite (branch/loop/return shapes) feeding the same engine — the Semgrep
  shape adopted incrementally, for only the languages that justify it — not a
  rewrite.
