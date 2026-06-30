//! The regex-backed rules. Each is a [`RegexRule`]: a compiled [`Regex`] plus a
//! `judge` closure that turns a match into the text to report (or `None` to skip a
//! benign match, e.g. a `font-family` already set to a CSS variable). The raw
//! pattern is also handed to the engine's `RegexSet` prefilter.

use regex::{Captures, Regex};

use super::{LineHit, Rule};

/// Web/style source extensions — where colors, fonts, motion and inline SVG live.
const WEB_EXTS: &[&str] = &[
    "css", "scss", "sass", "less", "ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte", "html",
    "htm",
];

/// Component-source extensions — where hand-rolled inline `<svg>` shows up.
const COMPONENT_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte"];

type Judge = fn(&Captures) -> Option<String>;

pub struct RegexRule {
    id: &'static str,
    message: &'static str,
    exts: &'static [&'static str],
    pattern: String,
    re: Regex,
    judge: Judge,
}

impl RegexRule {
    fn new(
        id: &'static str,
        message: &'static str,
        exts: &'static [&'static str],
        pattern: &str,
        judge: Judge,
    ) -> Self {
        let re = Regex::new(pattern).expect("built-in rule pattern must compile");
        Self {
            id,
            message,
            exts,
            pattern: pattern.to_string(),
            re,
            judge,
        }
    }
}

impl Rule for RegexRule {
    fn id(&self) -> &'static str {
        self.id
    }

    fn message(&self) -> &'static str {
        self.message
    }

    fn applies_to_ext(&self, ext: &str) -> bool {
        self.exts.contains(&ext)
    }

    fn scan_line(&self, line: &str, out: &mut Vec<LineHit>) {
        for caps in self.re.captures_iter(line) {
            let whole = caps.get(0).expect("group 0 always present");
            if let Some(text) = (self.judge)(&caps) {
                out.push(LineHit {
                    col: whole.start() + 1,
                    matched: text,
                });
            }
        }
    }

    fn prefilter(&self) -> Option<&str> {
        Some(&self.pattern)
    }
}

/// Report the whole match verbatim.
fn whole(caps: &Captures) -> Option<String> {
    Some(caps[0].to_string())
}

/// A `font-family` value is fine when it's a CSS variable or a global keyword —
/// flag only inline literal font stacks (`font-family: Inter, sans-serif`).
fn judge_font(caps: &Captures) -> Option<String> {
    let raw = caps.get(1)?.as_str().trim();
    let bare = raw
        .trim_matches(|c| c == '"' || c == '\'' || c == ',')
        .trim();
    let lower = bare.to_ascii_lowercase();
    let is_var = lower.starts_with("var(");
    let is_keyword = matches!(
        lower.as_str(),
        "inherit" | "initial" | "unset" | "revert" | ""
    );
    if is_var || is_keyword {
        None
    } else {
        Some(raw.to_string())
    }
}

/// Report a motion declaration without its trailing colon (`transition`, not
/// `transition:`).
fn judge_motion(caps: &Captures) -> Option<String> {
    Some(caps[0].trim_end_matches([' ', ':']).to_string())
}

pub fn pattern_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(RegexRule::new(
            "hex-color",
            "hardcoded hex color literal — use a theme token / CSS variable so it stays themeable.",
            WEB_EXTS,
            r"#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{4}|[0-9a-fA-F]{3})\b",
            whole,
        )),
        Box::new(RegexRule::new(
            "inline-svg",
            "inline <svg> in component code — extract it into a named, reusable icon component.",
            COMPONENT_EXTS,
            r#"<svg[\s/>]|createElement\(\s*["']svg["']"#,
            whole,
        )),
        Box::new(RegexRule::new(
            "inline-font",
            "inline font-family stack — define the font once and reference a CSS variable.",
            WEB_EXTS,
            r"(?i)(?:font-family|fontFamily)\s*:\s*([^;}\n]+)",
            judge_font,
        )),
        Box::new(RegexRule::new(
            "motion",
            "ad-hoc transition/animation — centralize motion so it can be tuned or disabled.",
            WEB_EXTS,
            r"\b(?:transition|animation)(?:-[a-z-]+)?\s*:|@keyframes\b",
            judge_motion,
        )),
    ]
}
