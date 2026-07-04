//! Scan configuration assembled from CLI flags and handed to the [`Engine`].

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

/// Filenames a checked-in config may use, in priority order.
pub const CONFIG_NAMES: [&str; 2] = [".straitjacket.yaml", ".straitjacket.yml"];

/// Settings loaded from a `.straitjacket.yaml` checked into a repo. Every field
/// is optional and mirrors a CLI flag one-for-one (kebab-case keys). A missing
/// field falls back to the matching CLI flag, then the built-in default; a CLI
/// flag always wins over the file.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct FileConfig {
    /// Paths to scan (mirrors positional args).
    pub paths: Option<Vec<String>>,
    /// Output format: `text` or `json`.
    pub format: Option<String>,
    /// Run only these rule ids.
    pub only: Option<Vec<String>>,
    /// Skip these rule ids.
    pub skip: Option<Vec<String>>,
    /// `file-size` line budget (0 disables the rule).
    pub max_lines: Option<usize>,
    /// `deep-nesting` indentation-depth budget (0 disables the rule).
    pub max_nesting: Option<usize>,
    /// `slop-prose` density window, in characters.
    pub prose_window: Option<usize>,
    /// `duplication` minimum clone size, in tokens.
    pub dup_min_tokens: Option<usize>,
    /// Also scan `.json` files.
    pub include_json: Option<bool>,
    /// Don't respect `.gitignore` / hidden-file conventions.
    pub no_ignore: Option<bool>,
    /// Report findings but always exit 0.
    pub no_fail: Option<bool>,
}

/// Walk up from `start` (inclusive) looking for a config file; return the first
/// one found. This is what makes a checked-in `.straitjacket.yaml` picked up
/// automatically from anywhere inside the repo.
pub fn find_config(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        for name in CONFIG_NAMES {
            let candidate = d.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        dir = d.parent();
    }
    None
}

/// Read and parse a config file. Errors on malformed YAML or unknown keys — a
/// typo'd setting is a mistake worth surfacing, not silently ignoring.
pub fn load_config(path: &Path) -> anyhow::Result<FileConfig> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Default line budget for the `file-size` rule. LLMs tend to produce sprawling
/// single files; 1500 lines is a generous ceiling before that's worth a look.
pub const DEFAULT_MAX_LINES: usize = 1500;

/// Default nesting budget for the `deep-nesting` rule. Beyond ~6 levels of block
/// indentation a reader has to hold too many contexts at once; it's a warn, not a
/// hard fail, since parsers and state machines legitimately go deeper.
pub const DEFAULT_MAX_NESTING: usize = 6;

/// Default sliding-window size (in bytes/chars) for the `slop-prose` density check.
pub const DEFAULT_PROSE_WINDOW: usize = 400;

/// Default minimum token run for the `duplication` rule to count as a clone.
pub const DEFAULT_DUP_MIN_TOKENS: usize = 50;

#[derive(Debug, Clone)]
pub struct Config {
    /// Line budget for the `file-size` rule. `None` disables the rule.
    pub max_lines: Option<usize>,
    /// Indentation-depth budget for the `deep-nesting` rule. `None` disables it.
    pub max_nesting: Option<usize>,
    /// Run the `slop-prose` analyzer. On by default — straitjacket runs at its max
    /// and you ratchet down with `--skip slop-prose`. Tier-0 artifacts hard-fail;
    /// Tier 1–3 density warns/fails.
    pub slop_prose: bool,
    /// Sliding-window size for the `slop-prose` density check.
    pub prose_window: usize,
    /// Run the compiled-in `duplication` (copy/paste) detector. On by default.
    pub duplication: bool,
    /// Minimum token run for `duplication` to count a clone.
    pub dup_min_tokens: usize,
    /// Skip `.json` files. On by default — JSON is usually generated/config data,
    /// not human-written prose or code. Turn off to scan it too.
    pub skip_json: bool,
    /// `one-component`: at most one React component per `.tsx`/`.jsx` file.
    pub one_component: bool,
    /// `effect-in-component`: no `useEffect` in a file that declares a component.
    pub effect_in_component: bool,
    /// `prop-drilling`: a component's prop must not be forwarded unchanged into a
    /// child component (keep every component within one hop of its data).
    pub prop_drilling: bool,
    /// `store-passthrough`: a `use*Store` value must not be forwarded unchanged into
    /// a child component (the child should read the store directly).
    pub store_passthrough: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_lines: Some(DEFAULT_MAX_LINES),
            max_nesting: Some(DEFAULT_MAX_NESTING),
            slop_prose: true,
            prose_window: DEFAULT_PROSE_WINDOW,
            duplication: true,
            dup_min_tokens: DEFAULT_DUP_MIN_TOKENS,
            skip_json: true,
            one_component: true,
            effect_in_component: true,
            prop_drilling: true,
            store_passthrough: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kebab_case_keys() {
        let yaml = "
paths: [src, tests]
format: json
skip: [motion, slop-prose]
max-lines: 800
prose-window: 600
dup-min-tokens: 80
include-json: true
no-fail: true
";
        let c: FileConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            c.paths.as_deref(),
            Some(&["src".into(), "tests".into()][..])
        );
        assert_eq!(c.format.as_deref(), Some("json"));
        assert_eq!(c.max_lines, Some(800));
        assert_eq!(c.prose_window, Some(600));
        assert_eq!(c.dup_min_tokens, Some(80));
        assert_eq!(c.include_json, Some(true));
        assert_eq!(c.no_fail, Some(true));
        assert_eq!(c.no_ignore, None);
    }

    #[test]
    fn empty_config_is_all_none() {
        let c: FileConfig = serde_yaml::from_str("{}").unwrap();
        assert!(c.paths.is_none());
        assert!(c.max_lines.is_none());
        assert!(c.skip.is_none());
    }

    #[test]
    fn rejects_unknown_keys() {
        // snake_case is a typo — we use kebab-case — so this must error.
        assert!(serde_yaml::from_str::<FileConfig>("max_lines: 800").is_err());
        assert!(serde_yaml::from_str::<FileConfig>("bogus: 1").is_err());
    }
}
