# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Nothing yet — this is the top of the 0.2.0 development cycle.

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

[Unreleased]: https://github.com/zmaril/straitjacket/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/zmaril/straitjacket/releases/tag/v0.1.0
