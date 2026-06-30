# straitjacket

A fast, deterministic scanner that flags weird code LLMs tend to generate — before it lands. Think of it as a secret-scanner, but for slop: it sweeps your source with a set of pattern rules and fails CI when it finds the tells.

It's a single static Rust binary with no runtime dependencies, so it drops into any repo's CI regardless of language or stack.

## What it catches

The built-in rules are intentionally **generic** — no framework or single-language assumptions — so the same binary works across all your repos:

| rule | flags |
|------|-------|
| `emoji` | emoji glyphs in code/comments/strings (a reliable LLM tell). Color emoji, VS16-presented glyphs, and flag sequences — but **not** text symbols like `©` `™` `✓`, arrows, dashes, or the geometric star. Markdown is **not** scanned unless you pass `--emoji-markdown` (see below). |
| `hex-color` | hardcoded hex color literals (`#1e1e1e`) — use a theme token / CSS variable instead. |
| `inline-svg` | hand-rolled inline `<svg>` in component code — extract it into a named, reusable icon. |
| `inline-font` | inline `font-family` stacks — define the font once and reference a CSS variable. |
| `motion` | ad-hoc `transition` / `animation` / `@keyframes` — centralize motion so it can be tuned or disabled. |
| `file-size` | files longer than the line budget (default **1500**) — sprawling single files are a common LLM tell. Tune with `--max-lines`, disable with `--max-lines 0` or `--skip file-size`. |

Each rule only looks at the file types where it makes sense (e.g. `hex-color` ignores `.json`, `inline-svg` only scans component sources).

### Markdown emoji

`--emoji-markdown` is **off by default** but worth turning on: docs and READMEs are exactly where stray LLM emoji are often most unwelcome. When set, the `emoji` rule also scans `.md` / `.markdown` / `.mdx`:

```sh
straitjacket --emoji-markdown
```

Run `straitjacket --list-rules` to see them with descriptions.

## Install

```sh
cargo install --git https://github.com/zmaril/straitjacket
# or build locally:
cargo build --release   # ./target/release/straitjacket
```

## Usage

```sh
straitjacket                      # scan the current directory (honours .gitignore)
straitjacket src tests            # scan specific paths
straitjacket --format json        # machine-readable output
straitjacket --only emoji,hex-color
straitjacket --skip motion
straitjacket --emoji-markdown      # also flag emoji in .md / .markdown / .mdx
straitjacket --max-lines 800       # tighter file-size budget (0 disables the rule)
straitjacket --no-ignore          # don't respect .gitignore / hidden-file rules
straitjacket --no-fail            # report but always exit 0
```

Output is `path:line:col  [rule]  matched`. The process exits **1** when there are findings (so CI fails), **0** when clean.

## Suppressing a false positive

There are two scopes of escape hatch. Both just look for the marker text — the comment syntax (`//`, `#`, `/* */`, `<!-- -->`) doesn't matter.

**One line** — add a same-line comment:

```ts
const brandColor = "#ff6600"; // straitjacket-allow: fixed brand color, not themeable
```

- `straitjacket-allow` suppresses **every** rule on that line.
- `straitjacket-allow:<rule>` suppresses only that rule, e.g. `straitjacket-allow:hex-color`.

**A whole file** — put the marker on any one line of it (top of file is conventional). This is the right tool for a theme/palette file full of legitimate hexes:

```css
/* straitjacket-allow-file:hex-color  design tokens — hexes live here */
:root { --bg: #1e1e1e; --fg: #abb2bf; }
```

- `straitjacket-allow-file` exempts **every** rule for the file.
- `straitjacket-allow-file:<rule>` exempts only that rule for the file — so the palette above still gets checked for emoji, oversized length, etc.

### Ignoring big files

`file-size` is a whole-file rule, so use the file-scoped marker (a per-line `straitjacket-allow` won't silence it):

- **Exempt one file:** `straitjacket-allow-file:file-size` on any line of it:
  ```ts
  // straitjacket-allow-file:file-size  generated, intentionally large
  ```
- **Stop scanning generated files entirely:** add them to `.gitignore` or `.ignore` — straitjacket honors both. `.ignore` is handy for files you commit but don't want any tooling to lint, and it exempts the file from *all* rules, which is usually what you want for generated output.
- **Globally:** `--max-lines N` to raise the budget, or `--max-lines 0` / `--skip file-size` to turn it off.

## CI

Use the bundled GitHub Action in any repo:

```yaml
# .github/workflows/straitjacket.yml
name: straitjacket
on: [push, pull_request]
jobs:
  scan:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: zmaril/straitjacket@v0.1.0
        # with:
        #   args: "src --skip motion"
```

The action downloads the prebuilt binary and runs it over the repo, failing the job on any finding.

## Why these rules

These started life as the per-repo `lint-*` Bun scripts in [powdermonkey](https://github.com/zmaril/powdermonkey) (PR #41), written because Biome has no custom-rule support and an ESLint stack was too much machinery for a handful of deterministic checks. straitjacket extracts the **generic** subset — the ones that aren't tied to one project's design system — into a single tool meant to run everywhere. Project-specific checks (design-token vocabularies, framework conventions) deliberately stay in their own repos.

## See also

For another high-signal LLM tell, pair straitjacket with **[jscpd](https://github.com/kucherenko/jscpd)** — a copy/paste detector that flags duplicated code blocks across many languages. LLMs love to clone-and-tweak instead of factoring out a shared helper, so a duplication scan catches a class of slop straitjacket's pattern rules don't.

## License

MIT
