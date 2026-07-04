//! The `deep-nesting` whole-file rule: flag lines indented past a nesting budget.
//!
//! Depth is read straight off leading whitespace. That's reliable *because a
//! formatter is enforced* — straitjacket's working assumption — so canonical
//! indentation means one level of block nesting is exactly one indent unit, with no
//! need to tokenize the language. Tab-indented files count leading tabs directly;
//! space-indented files divide the leading-space width by a per-file unit detected
//! from the most common single-step increase in indentation.
//!
//! It's a whole-file rule (like `file-size`), not a per-line [`Rule`](crate::rules::Rule),
//! because the unit has to be detected from the whole file before any line's depth
//! is known. Findings are per line, so a line-scoped `straitjacket-allow` on the
//! offending line silences it (the engine applies that filter).

use std::collections::HashMap;

use crate::finding::{Finding, Severity};

/// Id of the whole-file nesting rule.
pub const DEEP_NESTING_ID: &str = "deep-nesting";

/// Extensions where leading indentation tracks block nesting. Deliberately just
/// programming languages — markup and data (`.md`, `.json`, `.yaml`, `.html`) and
/// stylesheets nest deeply by nature, so indentation there isn't the same smell.
pub const NEST_EXTS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "rb", "go", "rs", "java", "kt", "kts", "swift",
    "c", "h", "cc", "cpp", "hpp", "cs", "php", "scala", "sh", "bash", "zsh",
];

/// A line's leading whitespace, or `None` if the line is blank / whitespace-only
/// (a blank line carries no nesting information).
fn leading(line: &str) -> Option<&str> {
    let ws_len = line.len() - line.trim_start().len();
    if ws_len == line.len() {
        None
    } else {
        Some(&line[..ws_len])
    }
}

/// Whether the file is tab-indented: more indented lines begin with a tab than with
/// a space.
fn tab_indented(text: &str) -> bool {
    let (mut tabs, mut spaces) = (0usize, 0usize);
    for line in text.lines() {
        match leading(line).and_then(|w| w.chars().next()) {
            Some('\t') => tabs += 1,
            Some(_) => spaces += 1,
            None => {}
        }
    }
    tabs > spaces
}

/// The space-indent unit: the most common single-step *increase* in leading-space
/// width between consecutive code lines. A formatter opens one block at a time, so
/// that step is one nesting level; alignment/continuation lines produce off-unit
/// steps but they're never the mode. Falls back to 4 when there's nothing to go on.
fn space_unit(text: &str) -> usize {
    let mut counts: HashMap<usize, usize> = HashMap::new();
    let mut prev: Option<usize> = None;
    for line in text.lines() {
        // Only pure-space indentation contributes to unit detection.
        let Some(w) = leading(line).filter(|w| w.chars().all(|c| c == ' ')) else {
            continue;
        };
        if let Some(p) = prev {
            if w.len() > p {
                *counts.entry(w.len() - p).or_default() += 1;
            }
        }
        prev = Some(w.len());
    }
    // Most common step wins; tie-break to the smaller step; default 4.
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(step, _)| step)
        .filter(|&s| s > 0)
        .unwrap_or(4)
}

/// The nesting depth of a line's leading whitespace under the detected scheme.
fn depth_of(ws: &str, tabs: bool, unit: usize) -> usize {
    if tabs {
        ws.chars().take_while(|&c| c == '\t').count()
    } else {
        ws.chars().take_while(|&c| c == ' ').count() / unit
    }
}

/// Flag each line nested deeper than `max_nesting` levels. To stay quiet, a run of
/// consecutive over-budget lines reports only once, at the line where it first
/// crosses the budget; blank lines inside a run don't break it.
pub fn scan(text: &str, path: &str, max_nesting: usize) -> Vec<Finding> {
    let mut out = Vec::new();
    let tabs = tab_indented(text);
    let unit = if tabs { 1 } else { space_unit(text) };
    let mut in_run = false;
    for (idx, line) in text.lines().enumerate() {
        let Some(ws) = leading(line) else {
            continue; // blank line: no info, keep any current run open
        };
        let depth = depth_of(ws, tabs, unit);
        if depth > max_nesting {
            if !in_run {
                out.push(Finding {
                    rule: DEEP_NESTING_ID.to_string(),
                    path: path.to_string(),
                    line: idx + 1,
                    col: ws.len() + 1,
                    matched: format!("nesting depth {depth}"),
                    message: format!(
                        "line nested {depth} levels deep, over the {max_nesting}-level limit — deeply nested code is hard to follow; extract or flatten it."
                    ),
                    severity: Severity::Warning,
                });
                in_run = true;
            }
        } else {
            in_run = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depths(src: &str, max: usize) -> Vec<usize> {
        scan(src, "t", max).into_iter().map(|f| f.line).collect()
    }

    #[test]
    fn detects_two_space_unit() {
        // 4 levels deep at 2-space indent = 8 leading spaces; budget 3 → flagged.
        let src = "fn f() {\n  a\n    b\n      c\n        d\n}\n";
        let f = scan(src, "t", 3);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 5);
        assert_eq!(f[0].matched, "nesting depth 4");
    }

    #[test]
    fn detects_four_space_unit() {
        let src = "def f():\n    a\n        b\n            c\n";
        // widths 4,8,12 → unit 4; depths 1,2,3. Budget 2 → the depth-3 line flags.
        assert_eq!(depths(src, 2), vec![4]);
    }

    #[test]
    fn detects_tabs() {
        let src = "func f() {\n\ta\n\t\tb\n\t\t\tc\n}\n";
        // Depths 0,1,2,3,0; budget 1 → the run enters at the first depth-2 line and
        // reports once.
        let f = scan(src, "t", 1);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 3);
        assert_eq!(f[0].matched, "nesting depth 2");
    }

    #[test]
    fn one_finding_per_run() {
        // A block that stays over budget reports once, not per line.
        let src = "a\n  b\n    c\n      d\n      e\n      f\n";
        assert_eq!(depths(src, 2), vec![4]); // unit 2, first depth-3 line only
    }

    #[test]
    fn blank_lines_do_not_split_a_run() {
        let src = "a\n  b\n    c\n      d\n\n      e\n";
        assert_eq!(depths(src, 2), vec![4]);
    }

    #[test]
    fn shallow_file_is_clean() {
        let src = "fn f() {\n  a\n    b\n}\n";
        assert!(scan(src, "t", 3).is_empty());
    }
}
