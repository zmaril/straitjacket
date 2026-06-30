//! Emoji rule: flag emoji glyphs in code. LLMs love to decorate source, comments,
//! and log lines with color emoji — it renders inconsistently across platforms and
//! is a reliable tell. This is the most universal slop signal, so it runs on a broad
//! set of code/source extensions (but not docs like `.md`, where emoji are often
//! intentional).
//!
//! What counts as an emoji here is deliberately narrower than Unicode's broad
//! `Emoji` property, which also covers ASCII digits, `#`, `*`, and text-default
//! symbols such as © ® ™ that appear legitimately in code and license headers. We
//! flag a codepoint when it has *default emoji presentation* (a glyph that renders
//! in color on its own), or it's any emoji char forced into emoji presentation by a
//! following U+FE0F (the variation selector). Regional-indicator letters (used to
//! build flag emoji) are flagged too.

use super::{LineHit, Rule};
use unic_emoji_char::{is_emoji, is_emoji_presentation};

const VS16: char = '\u{FE0F}'; // variation selector-16: forces emoji presentation

/// Extensions the emoji rule scans by default. Code and stylesheet sources.
const EXTS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "css", "scss", "sass", "less", "vue", "svelte", "py",
    "rb", "go", "rs", "java", "kt", "kts", "swift", "c", "h", "cc", "cpp", "hpp", "cs", "php",
    "sh", "bash", "zsh", "sql",
];

/// Markdown/prose extensions — scanned only when emoji-in-markdown is enabled.
const MARKDOWN_EXTS: &[&str] = &["md", "markdown", "mdx"];

fn is_emoji_glyph(c: char, followed_by_vs16: bool) -> bool {
    // Regional indicators (used to build flag emoji).
    if ('\u{1F1E6}'..='\u{1F1FF}').contains(&c) {
        return true;
    }
    // Default-color emoji on their own (excludes text-default copyright/tm and digits).
    if is_emoji_presentation(c) {
        return true;
    }
    // Any emoji char (incl. text-default dingbats and keycap bases) explicitly
    // emoji-presented via a trailing variation selector.
    followed_by_vs16 && is_emoji(c)
}

pub struct EmojiRule {
    /// Whether to also scan Markdown files (off by default).
    markdown: bool,
}

impl EmojiRule {
    pub fn new(markdown: bool) -> Self {
        Self { markdown }
    }
}

impl Rule for EmojiRule {
    fn id(&self) -> &'static str {
        "emoji"
    }

    fn message(&self) -> &'static str {
        "emoji glyph in source — renders inconsistently across platforms; a common LLM tell. Remove it or use a text label/icon."
    }

    fn applies_to_ext(&self, ext: &str) -> bool {
        EXTS.contains(&ext) || (self.markdown && MARKDOWN_EXTS.contains(&ext))
    }

    fn scan_line(&self, line: &str, out: &mut Vec<LineHit>) {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        for (j, &(byte, c)) in chars.iter().enumerate() {
            // Don't report a bare VS16 — it always trails a glyph we already caught.
            if c == VS16 {
                continue;
            }
            let next_vs16 = chars.get(j + 1).is_some_and(|&(_, n)| n == VS16);
            if is_emoji_glyph(c, next_vs16) {
                let mut matched = String::from(c);
                if next_vs16 {
                    matched.push(VS16);
                }
                out.push(LineHit {
                    col: byte + 1,
                    matched,
                });
            }
        }
    }
}
