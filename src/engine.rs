//! The scan engine: holds the line rules and a single `RegexSet` built from the
//! regex-backed rules' prefilters. For each line it asks the `RegexSet` which
//! patterns could match (one pass over the line for all of them), then runs only
//! those rules' full scan plus any non-regex rules (the emoji scan). This is the
//! "match many patterns at once" shape, in pure Rust.
//!
//! On top of the per-line rules it also enforces one whole-file rule, `file-size`,
//! which doesn't fit the line-based [`Rule`] trait — it's evaluated from the line
//! count after the per-line pass.

use anyhow::Result;
use regex::RegexSet;

use crate::config::Config;
use crate::finding::Finding;
use crate::rules::{line_rules, LineHit, Rule};

/// Line-scoped escape hatch. `straitjacket-allow` on a line suppresses every rule
/// for that line; `straitjacket-allow:<id>` suppresses only the named rule.
const ALLOW: &str = "straitjacket-allow";

/// Whole-file escape hatch. `straitjacket-allow-file` anywhere in a file suppresses
/// every rule for the file; `straitjacket-allow-file:<id>` suppresses one rule. This
/// is how you exempt, say, a palette file from `hex-color` without per-line markers.
const ALLOW_FILE: &str = "straitjacket-allow-file";

/// Id of the whole-file line-count rule.
const FILE_SIZE_ID: &str = "file-size";

/// Extensions the `file-size` rule applies to — source, config and docs where a
/// huge single file is a smell (not lockfiles, data dumps, or binaries).
const SIZE_EXTS: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "css", "scss", "sass", "less", "vue", "svelte", "html",
    "htm", "py", "rb", "go", "rs", "java", "kt", "kts", "swift", "c", "h", "cc", "cpp", "hpp",
    "cs", "php", "sh", "bash", "zsh", "sql", "md", "markdown", "mdx", "json", "yaml", "yml",
    "toml",
];

pub struct Engine {
    rules: Vec<Box<dyn Rule>>,
    /// `RegexSet` over the prefilter patterns of the regex-backed rules.
    set: RegexSet,
    /// `set` pattern index → index into `rules`.
    set_to_rule: Vec<usize>,
    /// Indices into `rules` of rules with no prefilter (always run, e.g. emoji).
    nonregex: Vec<usize>,
    /// Per-rule enable flag, parallel to `rules`.
    enabled: Vec<bool>,
    /// `file-size` line budget, and whether the rule is enabled.
    max_lines: usize,
    file_size_enabled: bool,
}

impl Engine {
    /// Build an engine from a [`Config`].
    pub fn new(config: &Config) -> Result<Self> {
        let rules = line_rules(config.emoji_in_markdown);
        let mut patterns = Vec::new();
        let mut set_to_rule = Vec::new();
        let mut nonregex = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            match rule.prefilter() {
                Some(p) => {
                    patterns.push(p.to_string());
                    set_to_rule.push(i);
                }
                None => nonregex.push(i),
            }
        }
        let set = RegexSet::new(&patterns)?;
        let enabled = vec![true; rules.len()];
        Ok(Self {
            rules,
            set,
            set_to_rule,
            nonregex,
            enabled,
            max_lines: config.max_lines.unwrap_or(usize::MAX),
            file_size_enabled: config.max_lines.is_some(),
        })
    }

    /// All rule ids, in declaration order, ending with `file-size`.
    pub fn rule_ids(&self) -> Vec<&'static str> {
        let mut ids: Vec<&'static str> = self.rules.iter().map(|r| r.id()).collect();
        ids.push(FILE_SIZE_ID);
        ids
    }

    /// Human-facing description for a rule id, if present.
    pub fn message_for(&self, id: &str) -> Option<String> {
        if id == FILE_SIZE_ID {
            return Some(format!(
                "file longer than {} lines — sprawling single files are a common LLM tell; split it up.",
                self.max_lines
            ));
        }
        self.rules
            .iter()
            .find(|r| r.id() == id)
            .map(|r| r.message().to_string())
    }

    /// Restrict to exactly `ids` (others disabled). Returns ids that don't exist.
    pub fn keep_only(&mut self, ids: &[String]) -> Vec<String> {
        for (i, rule) in self.rules.iter().enumerate() {
            self.enabled[i] = ids.iter().any(|id| id == rule.id());
        }
        self.file_size_enabled = self.file_size_enabled && ids.iter().any(|id| id == FILE_SIZE_ID);
        self.unknown(ids)
    }

    /// Disable `ids`. Returns ids that don't exist.
    pub fn skip(&mut self, ids: &[String]) -> Vec<String> {
        for (i, rule) in self.rules.iter().enumerate() {
            if ids.iter().any(|id| id == rule.id()) {
                self.enabled[i] = false;
            }
        }
        if ids.iter().any(|id| id == FILE_SIZE_ID) {
            self.file_size_enabled = false;
        }
        self.unknown(ids)
    }

    fn unknown(&self, ids: &[String]) -> Vec<String> {
        let known = self.rule_ids();
        ids.iter()
            .filter(|id| !known.contains(&id.as_str()))
            .cloned()
            .collect()
    }

    /// Does any enabled rule look at this extension? Lets the caller skip reading
    /// files nothing will scan.
    pub fn handles_ext(&self, ext: &str) -> bool {
        self.rules
            .iter()
            .enumerate()
            .any(|(i, r)| self.enabled[i] && r.applies_to_ext(ext))
            || (self.file_size_enabled && SIZE_EXTS.contains(&ext))
    }

    /// Scan one file's text. `path` is the display path; `ext` is lowercased and
    /// dot-free.
    pub fn scan_text(&self, text: &str, path: &str, ext: &str) -> Vec<Finding> {
        let mut findings = Vec::new();

        // Which line rules are live for this file (enabled + applicable extension).
        let applies: Vec<bool> = self
            .rules
            .iter()
            .enumerate()
            .map(|(i, r)| self.enabled[i] && r.applies_to_ext(ext))
            .collect();
        let size_applies = self.file_size_enabled && SIZE_EXTS.contains(&ext);
        if applies.iter().all(|b| !b) && !size_applies {
            return findings;
        }

        // Whole-file `straitjacket-allow-file[:rule]` directives. Only pay for the
        // line-by-line scan when the marker is actually present somewhere.
        let file_allow = if text.contains(ALLOW_FILE) {
            FileAllow::scan(text)
        } else {
            FileAllow::default()
        };

        let mut total_lines = 0;
        for (idx, line) in text.lines().enumerate() {
            total_lines = idx + 1;
            let lineno = idx + 1;
            let allow = line_scope(line);

            // Regex-backed rules: the RegexSet tells us which could match this line.
            for set_i in self.set.matches(line).into_iter() {
                let ri = self.set_to_rule[set_i];
                let id = self.rules[ri].id();
                if applies[ri] && !file_allow.covers(id) && !scope_covers(&allow, id) {
                    self.collect(ri, line, lineno, path, &mut findings);
                }
            }

            // Non-regex rules (emoji): always evaluated when applicable.
            for &ri in &self.nonregex {
                let id = self.rules[ri].id();
                if applies[ri] && !file_allow.covers(id) && !scope_covers(&allow, id) {
                    self.collect(ri, line, lineno, path, &mut findings);
                }
            }
        }

        // Whole-file rule: line count over budget.
        if size_applies && !file_allow.covers(FILE_SIZE_ID) && total_lines > self.max_lines {
            findings.push(Finding {
                rule: FILE_SIZE_ID.to_string(),
                path: path.to_string(),
                line: self.max_lines + 1,
                col: 1,
                matched: format!("{total_lines} lines"),
                message: format!(
                    "file has {total_lines} lines, over the {}-line limit — sprawling single files are a common LLM tell; split it up.",
                    self.max_lines
                ),
            });
        }

        findings
    }

    fn collect(&self, ri: usize, line: &str, lineno: usize, path: &str, out: &mut Vec<Finding>) {
        let rule = &self.rules[ri];
        let mut hits: Vec<LineHit> = Vec::new();
        rule.scan_line(line, &mut hits);
        for hit in hits {
            out.push(Finding {
                rule: rule.id().to_string(),
                path: path.to_string(),
                line: lineno,
                col: hit.col,
                matched: hit.matched,
                message: rule.message().to_string(),
            });
        }
    }
}

/// What a `straitjacket-allow[-file]` marker covers.
enum Scope {
    None,
    /// Bare marker — every rule.
    All,
    /// `…:<id>` — only this rule.
    Only(String),
}

fn scope_covers(scope: &Scope, id: &str) -> bool {
    match scope {
        Scope::None => false,
        Scope::All => true,
        Scope::Only(only) => only == id,
    }
}

/// Parse the `:<id>` suffix (if any) that follows a marker keyword.
fn scope_from_rest(rest: &str) -> Scope {
    if let Some(after) = rest.strip_prefix(':') {
        let id: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if id.is_empty() {
            Scope::All
        } else {
            Scope::Only(id)
        }
    } else {
        Scope::All
    }
}

/// The line-scoped `straitjacket-allow[:rule]` directive on this line, if any. The
/// longer `straitjacket-allow-file` token is deliberately not treated as a line
/// marker (it's whole-file), so occurrences of it are skipped.
fn line_scope(line: &str) -> Scope {
    let mut start = 0;
    while let Some(rel) = line[start..].find(ALLOW) {
        let pos = start + rel;
        let rest = &line[pos + ALLOW.len()..];
        if rest.starts_with("-file") {
            start = pos + ALLOW.len();
            continue;
        }
        return scope_from_rest(rest);
    }
    Scope::None
}

/// The whole-file `straitjacket-allow-file[:rule]` directive on this line, if any.
fn file_scope(line: &str) -> Scope {
    match line.find(ALLOW_FILE) {
        Some(pos) => scope_from_rest(&line[pos + ALLOW_FILE.len()..]),
        None => Scope::None,
    }
}

/// Aggregated whole-file exemptions: which rules (or all of them) a file opts out of
/// via `straitjacket-allow-file` markers anywhere in its text.
#[derive(Default)]
struct FileAllow {
    all: bool,
    rules: Vec<String>,
}

impl FileAllow {
    fn scan(text: &str) -> Self {
        let mut fa = FileAllow::default();
        for line in text.lines() {
            match file_scope(line) {
                Scope::None => {}
                Scope::All => fa.all = true,
                Scope::Only(id) => fa.rules.push(id),
            }
        }
        fa
    }

    fn covers(&self, id: &str) -> bool {
        self.all || self.rules.iter().any(|r| r == id)
    }
}
