//! `slop-prose` — a whole-text analyzer for the linguistic tells of LLM-written
//! prose. Unlike the code rules, it does NOT fit the line-based `Rule` trait: it
//! reads a whole document and reasons about *density*.
//!
//! Two mechanisms, matching the research (see `notes/detectability-tiers.md`):
//!
//! - **Tier 0 — machine artifacts** (`oaicite`, `utm_source=chatgpt.com`,
//!   "As an AI language model", unfilled placeholders, …). These are near-certain
//!   copy-paste residue, so a single hit is an **Error** (hard fail) regardless of
//!   document length. Weight is irrelevant — they don't accumulate, they trigger.
//!
//! - **Tiers 1–3 — style smells** (AI-vocab words, stock phrases, negative
//!   parallelisms, spaced em dashes, curly quotes). Each carries a **weight**. No
//!   single one means much — the signal is *co-occurrence and density*. We slide a
//!   fixed `window`-byte span across the text and take the densest span's
//!   `score / window`. Dividing by the fixed window (not the actual span) makes
//!   short texts naturally lenient, so a one-line "not X, but Y" can't blow up the
//!   ratio. Elevated density → **Warning**; high density → **Error**.
//!
//! On by default; runs on prose extensions (Markdown and HTML) only, never code.
//! Ratchet it down with `--skip slop-prose`.
//!
//! The style smells (wordlist, stock phrases, templates) are **English only** — we
//! don't yet know what LLM slop reads like in other languages. Adding a language
//! means someone who reads it verifying which words/phrases actually sound sloppy;
//! that's an issue request, not a guess. The Tier-0 artifacts are language-agnostic.

use regex::Regex;

use crate::finding::{line_col, Finding, Severity};

const RULE: &str = "slop-prose";

/// Extensions treated as prose: Markdown docs and HTML. Not code.
pub const PROSE_EXTS: &[&str] = &["md", "markdown", "mdx", "html", "htm"];

/// Densest-window density at/above which the run fails, and the lower bound at which
/// it only warns. Density = summed smell weight within any `window`-byte span,
/// divided by the window. These are v1 calibration guesses — tune with real corpora.
const FAIL_DENSITY: f64 = 0.06;
const WARN_DENSITY: f64 = 0.03;

/// A style smell that contributes `weight` points wherever it matches.
struct Scored {
    re: Regex,
    weight: u32,
}

/// A Tier-0 artifact: any match is a hard fail.
struct Artifact {
    re: Regex,
    what: &'static str,
}

pub struct SlopProse {
    window: usize,
    artifacts: Vec<Artifact>,
    scored: Vec<Scored>,
}

impl SlopProse {
    pub fn new(window: usize) -> Self {
        SlopProse {
            window: window.max(1),
            artifacts: artifacts(),
            scored: scored(),
        }
    }

    /// Analyze one document. `path` is the display path. When `suppress` is false the
    /// per-line allow markers are ignored, yielding the raw candidate findings the engine
    /// uses to decide which markers actually suppressed something.
    pub fn scan(&self, text: &str, path: &str, suppress: bool) -> Vec<Finding> {
        let mut out = Vec::new();

        // Tier 0: any artifact match hard-fails, unless its line is allow-marked.
        for art in &self.artifacts {
            for m in art.re.find_iter(text) {
                let (line, col) = line_col(text, m.start());
                if suppress && line_allow_marked(text, m.start()) {
                    continue;
                }
                out.push(Finding {
                    rule: RULE.to_string(),
                    path: path.to_string(),
                    line,
                    col,
                    matched: m.as_str().to_string(),
                    message: format!(
                        "LLM output artifact ({}) — near-certain machine residue.",
                        art.what
                    ),
                    severity: Severity::Error,
                });
            }
        }

        // Tiers 1–3: collect weighted hits, then find the densest window.
        let mut hits: Vec<(usize, u32, String)> = Vec::new();
        for s in &self.scored {
            for m in s.re.find_iter(text) {
                hits.push((m.start(), s.weight, m.as_str().to_string()));
            }
        }
        hits.sort_by_key(|h| h.0);

        if let Some((sum, lo, hi)) = densest_window(&hits, self.window) {
            let density = sum as f64 / self.window as f64;
            let severity = if density >= FAIL_DENSITY {
                Some(Severity::Error)
            } else if density >= WARN_DENSITY {
                Some(Severity::Warning)
            } else {
                None
            };
            if let Some(severity) = severity {
                let (line, col) = line_col(text, hits[lo].0);
                let contributors = summarize(&hits[lo..hi]);
                out.push(Finding {
                    rule: RULE.to_string(),
                    path: path.to_string(),
                    line,
                    col,
                    matched: format!("density {density:.3} (score {sum}/{})", self.window),
                    message: format!(
                        "AI-prose density {density:.3} over a {}-char window — reads like LLM slop: {contributors}",
                        self.window
                    ),
                    severity,
                });
            }
        }

        out
    }
}

/// The maximum total weight of hits contained in any `window`-byte span, returned as
/// `(sum, lo_index, hi_index_exclusive)` into the (offset-sorted) `hits`. The optimal
/// window can always be taken to start at some hit, so a two-pointer sweep suffices.
fn densest_window(hits: &[(usize, u32, String)], window: usize) -> Option<(u32, usize, usize)> {
    if hits.is_empty() {
        return None;
    }
    let mut best = (0u32, 0usize, 0usize);
    let mut lo = 0usize;
    let mut sum = 0u32;
    for hi in 0..hits.len() {
        sum += hits[hi].1;
        while hits[hi].0 - hits[lo].0 >= window {
            sum -= hits[lo].1;
            lo += 1;
        }
        if sum > best.0 {
            best = (sum, lo, hi + 1);
        }
    }
    Some(best)
}

/// Join the distinct matched texts in a window into a short, readable list.
fn summarize(window: &[(usize, u32, String)]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for (_, _, label) in window {
        let l = label.trim();
        if !seen.iter().any(|s| s.eq_ignore_ascii_case(l)) {
            seen.push(l);
        }
    }
    let shown = 6.min(seen.len());
    let mut s = seen[..shown]
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(", ");
    if seen.len() > shown {
        s.push_str(&format!(", +{} more", seen.len() - shown));
    }
    s
}

/// Is the line containing byte `off` marked with a `straitjacket-allow`? Lets a doc
/// legitimately quote an artifact (like these notes do) without hard-failing.
fn line_allow_marked(text: &str, off: usize) -> bool {
    let start = text[..off.min(text.len())]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let end = text[off.min(text.len())..]
        .find('\n')
        .map(|i| off + i)
        .unwrap_or(text.len());
    text[start..end].contains("straitjacket-allow")
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("built-in slop-prose pattern must compile")
}

fn artifacts() -> Vec<Artifact> {
    let a = |what: &'static str, pat: &str| Artifact { re: re(pat), what };
    vec![
        a("citation residue", r"contentReference|oaicite|oai_citation"),
        a(
            "Grok card markup",
            r"grok_render_citation_card_json|grok_card",
        ),
        a(
            "search/image token",
            r"turn\d+(?:search|image|news|file)\d+",
        ),
        a(
            "tracking parameter",
            r"utm_source=(?:chatgpt\.com|openai|copilot\.com)|referrer=grok\.com",
        ),
        a(
            "attribution JSON",
            r#"\{"attribution":\{"attributableIndex""#,
        ),
        a("document markup", r#":::writing\{variant"#),
        a("AI self-reference", r"(?i)\bas an ai language model\b"),
        a(
            "knowledge-cutoff disclaimer",
            r"(?i)as of my last knowledge (?:update|cutoff)",
        ),
        a(
            "unfilled placeholder",
            r"INSERT_[A-Z0-9_]+|PASTE_[A-Z0-9_]+|\[Your Name\]",
        ),
        a("placeholder date", r"\b\d{4}-[Xx]{2}-[Xx]{2}\b"),
    ]
}

fn scored() -> Vec<Scored> {
    let s = |weight: u32, pat: &str| Scored {
        re: re(pat),
        weight,
    };
    let mut v = Vec::new();

    // Tier 2 — stock phrases (strong, weight 8).
    for pat in [
        r"(?i)rich cultural heritage",
        r"(?i)rich tapestry",
        r"(?i)stands? as a testament",
        r"(?i)plays? a (?:vital|pivotal|crucial|significant) role",
        r"(?i)maintains an active social media presence",
        r"(?i)leaving a lasting (?:impact|impression|legacy)",
        r"(?i)nestled in the heart of",
        r"(?i)it'?s worth noting",
        r"(?i)in today'?s (?:fast-paced|digital|modern|ever-changing) (?:world|age|landscape|era)",
    ] {
        v.push(s(8, pat));
    }

    // Tier 3 — negative parallelisms and formulas (weight 5).
    for pat in [
        r"(?i)\bnot only\b[^.\n]{1,60}?\bbut(?: also)?\b",
        r"(?i)\bnot just\b[^.\n]{1,50}?\bbut\b",
        r"(?i)\b(?:it'?s|its|this is) not\b[^.\n]{1,50}?,?\s+(?:it'?s|its|but)\b",
        r"(?i)\bno [^,\n]{1,25}?, no [^,\n]{1,25}?, just\b",
        r"(?i)\bdespite [^.\n]{1,60}?,? [^.\n]{0,40}?faces? (?:several |numerous )?challenges\b",
    ] {
        v.push(s(5, pat));
    }

    // Tier 2 — AI-vocabulary words (weight 3). Curated to high-signal ones only.
    for pat in [
        r"(?i)\bdelv(?:e|ed|es|ing)\b",
        r"(?i)\btapestry\b",
        r"(?i)\btestament\b",
        r"(?i)\bmeticulous(?:ly)?\b",
        r"(?i)\bpivotal\b",
        r"(?i)\bunderscor(?:e|es|ed|ing)\b",
        r"(?i)\bintricat(?:e|ies)\b|\bintricacies\b",
        r"(?i)\bvibrant\b",
        r"(?i)\bshowcas(?:e|es|ed|ing)\b",
        r"(?i)\bfoster(?:s|ed|ing)?\b",
        r"(?i)\bnestled\b",
        r"(?i)\bmultifaceted\b",
        r"(?i)\bseamless(?:ly)?\b",
        r"(?i)\bboasts?\b",
        r"(?i)\bbolster(?:s|ed|ing)?\b",
    ] {
        v.push(s(3, pat));
    }

    // Tier 1 — formatting tics (low weight).
    v.push(s(2, r" \u{2014} ")); // spaced em dash
    v.push(s(1, r"[\u{201C}\u{201D}\u{2018}\u{2019}]")); // curly quotes/apostrophes

    v
}
