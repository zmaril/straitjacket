<!-- housekeeper:description no! more! slop! -->
<!-- housekeeper:topics agent, ai, ai-code-review, ai-detection, ai-slop-detection, amazing, ci, cli, code-quality, code-review, developer-tools, github-actions, impressive, linter, llm, rust, scanner, slop, static-analysis, wow -->
# straitjacket

<p align="center">
  <img src="assets/strait-waistcoat.jpg" alt="Engraving of a patient restrained in a strait-waistcoat" width="320">
  <br>
  <em><sub>Insane patient in a strait-waistcoat. Wellcome Collection (L0011301), <a href="https://creativecommons.org/licenses/by/4.0">CC BY 4.0</a>, via <a href="https://commons.wikimedia.org/wiki/File:Insane_patient_in_a_strait-waistcoat._Wellcome_L0011301.jpg">Wikimedia Commons</a>.</sub></em>
</p>

<p align="center">
  <a href="https://straitjacket.dev"><img src="https://img.shields.io/badge/docs-straitjacket.dev-111?logo=readthedocs&logoColor=white" alt="Documentation"></a>
  <a href="https://github.com/marketplace/actions/powderworks-straitjacket"><img src="https://img.shields.io/badge/marketplace-powderworks--straitjacket-2088FF?logo=githubactions&logoColor=white" alt="Powderworks Straitjacket on the GitHub Marketplace"></a>
  <a href="https://discord.gg/5G6KvdJffj"><img src="https://img.shields.io/badge/Discord-join%20the%20chat-5865F2?logo=discord&logoColor=white" alt="Join the Discord"></a>
  <a href="https://x.com/ZackMaril"><img src="https://img.shields.io/badge/X-%40ZackMaril-000?logo=x&logoColor=white" alt="Follow @ZackMaril on X"></a>
</p>

Straitjacket is a fast, deterministic scanner that flags the weird code and text LLMs like to produce. It sweeps your files against a set of rules — with snobby yet configurable defaults — and flags anything it finds. It's a single static Rust binary with no runtime dependencies, so it drops into almost any environment or repo's CI, regardless of language or stack.

> [!TIP]
> **Like Straitjacket?** Try out [**PowderMonkey**](https://github.com/zmaril/powdermonkey) — a (straitjacketed) agent orchestration harness for aspiring slop cannons.

## Docs

Full documentation — install, usage, every rule, CI setup, and the reasoning behind the checks — lives at **[straitjacket.dev](https://straitjacket.dev)**.

```sh
# quick start (Linux x86_64, macOS arm64/x86_64):
curl -fsSL https://raw.githubusercontent.com/zmaril/straitjacket/main/install.sh | sh
straitjacket
```

## Background & philosophy

Straitjacket started life as the per-repo `lint-*` Bun scripts in [powdermonkey](https://github.com/zmaril/powdermonkey) (PR #41), written because I got annoyed with the way Claude kept messing with the design of the interface, as well as with the kinds of code and text it would output. I'd written versions of these linters across various projects over the last few years, and I kept finding new smells as I generated more code and text over time. Eventually I decided to bundle them all into one tool, so I wouldn't have to keep rewriting them haphazardly all over the place — and so other people could use it and tell me what other annoying things LLMs tend to do.

During the initial development of Straitjacket, I had a strong realization: what bothers me most about the way LLMs change the design of an application maps neatly onto common UI settings. Claude randomly inserts elements and changes their colors — that's the province of a theme switcher. Claude decides it needs ten font families and a hundred sizes and weights — that's the purview of a font family and size picker. Every element on a page wiggles in its own individual way; well, well, well, that's a motion-control toggle. So, in a way, in lieu of guidance — of an enforced design system — why shouldn't Claude get freaky with it? We never said it couldn't.

So, alongside restricting the design tokens above to blessed files, I'd recommend giving users a way to control these settings too. To me, the two go hand in hand. Likewise, when reviewing code, I found it was very easy for Claude to squirrel thousands of lines away into a single file. I'd review all the lines, they'd look fine, but these monsters would sneak up on me before I knew it. Refactoring them always made the codebase better, and I've found that 1500 lines is about where they start breaking down logically enough for me to notice.

As for slop text, it just smells. There's no way around it, and I don't like it. Straitjacket does its best to scan for the most common signs. It's not wrong to use the word *delve*, but it does get suspicious when you use it often, alongside other signs. Not trying to get too fancy with it.

Straitjacket has become an exercise in me encoding as much of my personal tastes as I can into deterministic checkers I can run across LLM output, hopefully saving me the trouble of having to go "Yuck!" myself.

## Contributing

Found a new smell?

LLMs invent new tells constantly, and everyone's "Yuck!" is a little different. If you've spotted a pattern straitjacket should catch — or a false positive it shouldn't! — [**file an issue**](https://github.com/zmaril/straitjacket/issues). Concrete examples help most. Two things especially wanted:

- **New rules** — a deterministic smell that generalizes across repos.
- **`slop-prose` in another language** — if you read it and can verify what actually sounds sloppy, say so in the issue (the docs' [slop-prose page](https://straitjacket.dev/docs/explanation/slop-prose) explains why it's English-only for now).

Working on the code? Run `./scripts/dev.sh` once after cloning — it wires up the committed git hooks, builds the crate, and installs the docs site's dependencies.

Prefer to talk it through first? [**Join the Discord**](https://discord.gg/5G6KvdJffj), or follow [**@ZackMaril** on X](https://x.com/ZackMaril).

## License

Code is [MIT](LICENSE). Notable changes are tracked in the [changelog](CHANGELOG.md).

The banner image (`assets/strait-waistcoat.jpg`) — *Insane patient in a strait-waistcoat*, [Wellcome Collection](https://wellcomecollection.org/works/ckwscya3) (L0011301) — is licensed [CC BY 4.0](https://creativecommons.org/licenses/by/4.0) and is **not** covered by the MIT license; reuse it under its own terms.
