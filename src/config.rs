//! Scan configuration assembled from CLI flags and handed to the [`Engine`].

/// Default line budget for the `file-size` rule. LLMs tend to produce sprawling
/// single files; 1500 lines is a generous ceiling before that's worth a look.
pub const DEFAULT_MAX_LINES: usize = 1500;

#[derive(Debug, Clone)]
pub struct Config {
    /// Also scan Markdown for emoji. Off by default — Markdown is where emoji are
    /// most often deliberate (and most often unwanted), so it's opt-in either way.
    pub emoji_in_markdown: bool,
    /// Line budget for the `file-size` rule. `None` disables the rule.
    pub max_lines: Option<usize>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            emoji_in_markdown: false,
            max_lines: Some(DEFAULT_MAX_LINES),
        }
    }
}
