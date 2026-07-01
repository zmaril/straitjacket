import Link from "next/link";
import { DiscordIcon, XIcon } from "@/components/icons";

const RULES = [
  {
    title: "Emoji & AI tells",
    body: "Emoji glyphs in code, comments, strings, and Markdown — one of the most reliable giveaways that a machine wrote it.",
  },
  {
    title: "Hardcoded colors",
    body: "Raw hex and CSS color functions (rgb, hsl, oklch, …) that should be a theme token, not sprinkled inline.",
  },
  {
    title: "Oversized files",
    body: "The 1,500-line monsters that pass review one screen at a time and sneak up on you. Tunable, off with a flag.",
  },
  {
    title: "Slop prose",
    body: "The linguistic tells of AI-written Markdown — stock phrases, negative parallelisms, machine artifacts — scored by density.",
  },
  {
    title: "Copy-paste",
    body: "Clone-and-tweak duplication across the whole tree, detected in-process. A structure may appear only once.",
  },
  {
    title: "React smells",
    body: "Pure prop-drilling, stray effects, and more-than-one-component files — AST-based, inert outside React.",
  },
];

export default function HomePage() {
  return (
    <main className="flex flex-1 flex-col">
      {/* Hero: image on the left, pitch on the right */}
      <section className="mx-auto grid w-full max-w-6xl grid-cols-1 items-center gap-10 px-6 py-10 md:grid-cols-[minmax(0,1fr)_1.4fr] md:py-14">
        <figure className="mx-auto w-full max-w-[200px] md:justify-self-center lg:max-w-[240px]">
          {/* Plain <img>: the site is a static export, so next/image
              optimization isn't available. */}
          <img
            src="/strait-waistcoat.jpg"
            alt="Engraving of a patient restrained in a strait-waistcoat"
            className="w-full rounded-xl border shadow-sm"
            width={1148}
            height={1814}
          />
          <figcaption className="mt-3 text-center text-xs text-fd-muted-foreground">
            Insane patient in a strait-waistcoat. Wellcome Collection
            (L0011301),{" "}
            <a
              className="underline"
              href="https://creativecommons.org/licenses/by/4.0"
            >
              CC BY 4.0
            </a>
            .
          </figcaption>
        </figure>

        <div className="flex flex-col items-start text-left">
          <h1 className="text-4xl font-bold tracking-tight sm:text-5xl">
            Straitjacket
          </h1>
          <p className="mt-3 text-xl font-medium text-fd-foreground">
            A secret scanner, but for slop.
          </p>
          <p className="mt-4 text-fd-muted-foreground">
            Straitjacket is a fast, deterministic scanner that flags the weird
            code and text LLMs tend to generate. It sweeps your files against a
            set of snobby-but-configurable rules and flags anything it finds —
            one static Rust binary, no runtime, so it drops into any repo's CI
            regardless of language or stack.
          </p>

          <div className="mt-8 flex flex-wrap items-center gap-3">
            <Link
              href="/docs/tutorials/getting-started"
              className="rounded-full bg-fd-primary px-6 py-2.5 text-sm font-medium text-fd-primary-foreground transition-colors hover:bg-fd-primary/90"
            >
              Get started
            </Link>
            <Link
              href="/docs"
              className="rounded-full border px-6 py-2.5 text-sm font-medium transition-colors hover:bg-fd-accent"
            >
              Read the docs
            </Link>
            <a
              href="https://github.com/zmaril/straitjacket"
              className="rounded-full border px-6 py-2.5 text-sm font-medium transition-colors hover:bg-fd-accent"
            >
              GitHub
            </a>
            <a
              href="https://discord.gg/5G6KvdJffj"
              className="inline-flex items-center gap-2 rounded-full bg-[#5865F2] px-6 py-2.5 text-sm font-medium text-white transition-colors hover:bg-[#4752c4]"
            >
              <DiscordIcon width={18} height={18} />
              Discord
            </a>
            <a
              href="https://x.com/ZackMaril"
              className="inline-flex items-center gap-2 rounded-full border px-6 py-2.5 text-sm font-medium transition-colors hover:bg-fd-accent"
            >
              <XIcon width={16} height={16} />
              Follow
            </a>
          </div>

          <pre className="mt-8 w-full overflow-x-auto rounded-lg border bg-fd-card p-4 text-left text-sm text-fd-muted-foreground">
            <code>
              curl -fsSL
              https://raw.githubusercontent.com/zmaril/straitjacket/main/install.sh
              | sh
            </code>
          </pre>
        </div>
      </section>

      {/* PowderMonkey cross-promo */}
      <section className="mx-auto w-full max-w-6xl px-6 py-4">
        <a
          href="https://github.com/zmaril/powdermonkey"
          className="flex flex-col items-start gap-2 rounded-xl border border-fd-primary/30 bg-fd-primary/5 px-5 py-4 transition-colors hover:bg-fd-primary/10 sm:flex-row sm:items-center sm:justify-between"
        >
          <span className="text-sm sm:text-base">
            <span className="font-semibold">Like Straitjacket?</span> Try out{" "}
            <span className="font-semibold">PowderMonkey</span> — a
            (straitjacketed) agent orchestration harness for aspiring slop
            cannons.
          </span>
          <span className="whitespace-nowrap text-sm font-medium text-fd-primary">
            View on GitHub →
          </span>
        </a>
      </section>

      {/* What it catches */}
      <section className="mx-auto w-full max-w-6xl px-6 py-8">
        <h2 className="text-2xl font-semibold tracking-tight">
          What it catches
        </h2>
        <p className="mt-2 max-w-2xl text-fd-muted-foreground">
          Everything is on by default — Straitjacket runs at its max, and you
          ratchet down with <code className="text-sm">--skip</code>. Each rule
          only looks at the file types where it makes sense.
        </p>
        <div className="mt-8 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {RULES.map((rule) => (
            <div
              key={rule.title}
              className="rounded-xl border bg-fd-card p-5 transition-colors hover:bg-fd-accent/50"
            >
              <h3 className="font-semibold">{rule.title}</h3>
              <p className="mt-2 text-sm text-fd-muted-foreground">
                {rule.body}
              </p>
            </div>
          ))}
        </div>
      </section>

      {/* Example output */}
      <section className="mx-auto w-full max-w-6xl px-6 py-12">
        <div className="grid grid-cols-1 items-center gap-10 lg:grid-cols-2">
          <div>
            <h2 className="text-2xl font-semibold tracking-tight">
              One command. Every rule.
            </h2>
            <p className="mt-4 text-fd-muted-foreground">
              Run <code className="text-sm">straitjacket</code> at the root of
              any project. It honors your{" "}
              <code className="text-sm">.gitignore</code>, prints one line per
              finding as{" "}
              <code className="text-sm">path:line:col [rule] matched</code>, and
              exits non-zero on any error — so CI fails the moment slop lands.
            </p>
            <p className="mt-4 text-fd-muted-foreground">
              No config to write, no toolchain to install. Suppress a false
              positive on one line with{" "}
              <code className="text-sm">straitjacket-allow</code>, or a whole
              file with <code className="text-sm">straitjacket-allow-file</code>
              .
            </p>
          </div>
          <pre className="overflow-x-auto rounded-xl border bg-fd-card p-5 text-sm leading-relaxed">
            <code>
              <span className="text-fd-muted-foreground">$ </span>straitjacket
              {"\n\n"}
              src/theme.ts:42:7 <span className="text-fd-primary">[color]</span>{" "}
              #1e1e1e{"\n"}
              src/Dashboard.tsx:88:3{" "}
              <span className="text-fd-primary">[effect-in-component]</span>{" "}
              useEffect{"\n"}
              src/panes/GoalGroup.tsx:31:5{" "}
              <span className="text-fd-primary">[prop-drilling]</span> selection
              {"\n"}
              CHANGELOG.md:14:4{" "}
              <span className="text-fd-muted-foreground">
                [slop-prose] (warn)
              </span>{" "}
              density 0.100{"\n"}
              src/utils.ts:5:1{" "}
              <span className="text-fd-primary">[duplication]</span> 9 lines, 71
              tokens{"\n\n"}
              <span className="text-fd-muted-foreground">
                straitjacket: 4 error(s), 1 warning(s) across 128 file(s)
              </span>
            </code>
          </pre>
        </div>
      </section>

      {/* Closing CTA */}
      <section className="mx-auto w-full max-w-6xl px-6 py-16">
        <div className="flex flex-col items-center gap-6 rounded-2xl border bg-fd-card px-6 py-12 text-center">
          <h2 className="text-2xl font-semibold tracking-tight">
            Put your slop in a Straitjacket.
          </h2>
          <p className="max-w-xl text-fd-muted-foreground">
            Encode your taste as deterministic checks and run them across
            everything an LLM writes — so you never have to go "Yuck!" by hand
            again.
          </p>
          <div className="flex flex-wrap items-center justify-center gap-3">
            <Link
              href="/docs/tutorials/getting-started"
              className="rounded-full bg-fd-primary px-6 py-2.5 text-sm font-medium text-fd-primary-foreground transition-colors hover:bg-fd-primary/90"
            >
              Get started
            </Link>
            <Link
              href="/docs/reference/rules"
              className="rounded-full border px-6 py-2.5 text-sm font-medium transition-colors hover:bg-fd-accent"
            >
              Browse the rules
            </Link>
          </div>
        </div>
      </section>
    </main>
  );
}
