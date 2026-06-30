//! The rule abstraction and the built-in rule set.
//!
//! A [`Rule`] is a deterministic check over a single line of source. Most rules are
//! regex-backed (see [`patterns`]); the emoji rule scans codepoints directly (see
//! [`emoji`]). The [`Engine`](crate::Engine) drives them.

mod emoji;
mod patterns;

/// One occurrence found on a line: the 1-based byte column and the offending text.
pub struct LineHit {
    pub col: usize,
    pub matched: String,
}

/// A single deterministic check over a line of source.
pub trait Rule: Sync + Send {
    /// Stable identifier, e.g. `"emoji"`. Used in output and `--only`/`--skip`.
    fn id(&self) -> &'static str;

    /// One-line explanation shown with each finding.
    fn message(&self) -> &'static str;

    /// Does this rule look at files with the given (lowercased, no-dot) extension?
    fn applies_to_ext(&self, ext: &str) -> bool;

    /// Append every hit on `line` to `out`. The caller has already handled the
    /// `straitjacket-allow` escape hatch, so the rule need not check for it.
    fn scan_line(&self, line: &str, out: &mut Vec<LineHit>);

    /// A regex that *over-approximates* this rule's matches, used by the engine's
    /// `RegexSet` prefilter. `None` means the rule has no cheap prefilter and is
    /// always run. When `Some`, the engine only calls [`scan_line`](Rule::scan_line)
    /// for lines the prefilter flags, so the pattern must never miss a real hit.
    fn prefilter(&self) -> Option<&str> {
        None
    }
}

/// The built-in line-based rule set: emoji, hex colors, inline `<svg>`, inline
/// `font-family` stacks, and ad-hoc CSS motion. Order is stable (used in output).
/// (The whole-file `file-size` rule lives on the [`Engine`](crate::Engine), not
/// here, since it isn't a per-line check.)
pub fn line_rules(emoji_in_markdown: bool) -> Vec<Box<dyn Rule>> {
    let mut rules: Vec<Box<dyn Rule>> = vec![Box::new(emoji::EmojiRule::new(emoji_in_markdown))];
    rules.extend(patterns::pattern_rules());
    rules
}
