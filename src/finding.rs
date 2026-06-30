use serde::Serialize;

/// One flagged occurrence. Positions are 1-based; `col` is a byte offset into the
/// line (ripgrep convention), so it's stable regardless of multi-byte glyphs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Finding {
    /// Rule id that produced this, e.g. `"emoji"`.
    pub rule: String,
    /// Display path of the file (relative to the scan root when possible).
    pub path: String,
    pub line: usize,
    pub col: usize,
    /// The exact text that tripped the rule.
    pub matched: String,
    /// Human-facing explanation of why this is flagged.
    pub message: String,
}
