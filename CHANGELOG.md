# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.3] - 2026-07-04

### Fixed

- **`duplication` in Markdown** now honours `straitjacket-allow` markers too. A clone
  inside a doc's fenced code block carries a `:<lang>` tag on its source id (e.g.
  `docs.md:bash`), so the finding's path wasn't a real file — the suppression added in
  0.2.2 couldn't open it and the marker was ignored. The `:<lang>` tag is now stripped,
  which also tidies the reported path.

## [0.2.2] - 2026-07-04

### Fixed

- **`duplication`** findings now honour `straitjacket-allow` markers. The cross-file
  copy/paste pass runs separately from the per-file rules, so `allow-file:duplication`
  (and line-scoped `straitjacket-allow`) had no effect on a clone — you couldn't
  suppress a genuinely-coincidental duplicate, which blocked gating a repo on
  duplication. They're now filtered through the same suppression the other rules use.

## [0.2.1] - 2026-07-03

### Fixed

- **`inline-font`** no longer flags a *quoted* CSS variable — `fontFamily:
  "var(--mantine-font-family-monospace)"` is a token reference just like the bare
  `var(--x)` form, so both are allowed now. A quoted *font* (`"Inter"`) is still a
  hardcoded literal and stays flagged.

## [0.2.0] - 2026-07-03

### Added

- **SARIF output** — `--format sarif` emits SARIF 2.1.0 for GitHub code scanning,
  and a new `--sarif <PATH>` flag writes SARIF to a file *alongside* the normal
  stdout report (readable log **and** code-scanning upload in one pass). The GitHub
  Action now uploads SARIF **by default**: findings show as annotations on the PR
  diff and in the Security tab (grant `security-events: write`; without it the
  upload is skipped gracefully and the scan still gates). The job still fails on
  findings — `no-fail: "true"` opts out, `sarif: "false"` disables the upload.

### Changed

- **`effect-in-component`** is now scope-aware: it flags a `useEffect` only when it
  is defined **inside a component's body**, not merely present in a file that has a
  component. An effect inside a custom `use*` hook is fine — even in the same file as
  the component — so hooks can be co-located. Anonymous components (`memo`/`forwardRef`)
  still count.
- Upgrade the OXC crates (`oxc_parser`, `oxc_semantic`, and the rest) from 0.133
  to 0.138. The React rules now build the semantic node arena explicitly
  (`with_build_nodes(true)`), required as of 0.138.

### Fixed

- **`inline-font`** no longer flags a font *token reference* — `fontFamily: MONO`
  (a variable, the good pattern) and bare generic families (`monospace`,
  `sans-serif`) are allowed. Only a quoted font or a hardcoded multi-family stack
  (`Inter, sans-serif`) is flagged. Also fixes a false positive on a single-line JS
  object where the property-separator comma was read as a font fallback.

## [0.1.0] - 2026-07-01

### Added

- **Core scanner** — a single static Rust binary that walks a tree
  (`.gitignore`-aware) and flags weird code and text LLMs tend to generate. Text
  or JSON output; exits non-zero on any error-level finding so CI fails.
- **Generic rules** — `emoji`, `color` (hex and CSS color functions),
  `inline-svg`, `inline-font`, `motion`, and `file-size` (default 1500-line
  budget).
- **`slop-prose`** — an analyzer for AI-written prose in Markdown/HTML: copy-paste
  machine artifacts hard-fail, and a density gate over a sliding window warns then
  fails on a high concentration of style tells. English only, for now.
- **`duplication`** — compiled-in copy/paste detection via
  [`cpd-finder`](https://crates.io/crates/cpd-finder) (the engine behind jscpd 5);
  no external tool.
- **React rule tier (OXC)** — `one-component`, `effect-in-component`,
  `prop-drilling` (semantic pure-conduit detection), and `store-passthrough`, plus
  `--prop-chains` for cross-file drill-depth analysis.
- **Suppression markers** — `straitjacket-allow[:rule]` (line) and
  `straitjacket-allow-file[:rule]` (whole file).
- **Config file** — a checked-in `.straitjacket.yaml`, auto-discovered by walking
  up from the working directory; mirrors the CLI flags. `--config` / `--no-config`
  to override discovery.
- **CLI** — `--only` / `--skip`, `--max-lines`, `--prose-window`,
  `--dup-min-tokens`, `--include-json`, `--no-ignore`, `--no-fail`,
  `--list-rules`.
- **Distribution** — `curl | sh` installer, prebuilt binaries (Linux x86_64,
  macOS arm64/x86_64, Windows x86_64), and a GitHub composite Action with typed
  inputs.
- **Documentation** — a [straitjacket.dev](https://straitjacket.dev) docs site
  (Fumadocs), organized by Diátaxis.
- **License** — MIT.

[Unreleased]: https://github.com/zmaril/straitjacket/compare/v0.2.3...HEAD
[0.2.3]: https://github.com/zmaril/straitjacket/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/zmaril/straitjacket/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/zmaril/straitjacket/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/zmaril/straitjacket/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/zmaril/straitjacket/releases/tag/v0.1.0
