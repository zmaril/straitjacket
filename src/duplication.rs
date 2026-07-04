//! `duplication` — copy/paste detection compiled directly into straitjacket via the
//! `cpd-finder` library (the engine behind jscpd 5, which is itself Rust). No
//! external binary to install and no Node: straitjacket walks and tokenizes the
//! tree itself and reports every clone.
//!
//! The policy matches straitjacket's max-by-default stance: **a structure may appear
//! only once.** Any clone of at least `min_tokens` tokens is an `Error`. This is a
//! cross-file, whole-run analysis, so it runs once over the scan paths rather than
//! per file.

use std::path::PathBuf;

use cpd_finder::orchestrate::{run, RunConfig};

use crate::finding::{Finding, Severity};

const RULE: &str = "duplication";

/// Detect duplicated code across `paths` and return one finding per clone. `ignore`
/// holds extra glob patterns to exclude (e.g. `**/*.json`). Detection failures
/// degrade to an empty result rather than aborting the whole scan.
pub fn detect(
    paths: &[PathBuf],
    respect_ignore: bool,
    min_tokens: usize,
    ignore: &[String],
) -> Vec<Finding> {
    let config = RunConfig {
        paths: paths.to_vec(),
        min_tokens,
        no_gitignore: !respect_ignore,
        ignore: ignore.to_vec(),
        ..RunConfig::default()
    };
    let Ok(result) = run(&config) else {
        return Vec::new();
    };

    result
        .clones
        .into_iter()
        .map(|clone| {
            let a = clone.fragment_a;
            let b = clone.fragment_b;
            let lines = a.end.line.saturating_sub(a.start.line) + 1;
            Finding {
                rule: RULE.to_string(),
                path: tidy(&a.source_id),
                line: a.start.line as usize,
                col: a.start.column as usize,
                matched: format!("{lines} lines, {} tokens", clone.token_count),
                message: format!(
                    "duplicated code — this block also appears at {}:{}. LLMs clone-and-tweak; factor out a shared helper.",
                    tidy(&b.source_id),
                    b.start.line
                ),
                severity: Severity::Error,
            }
        })
        .collect()
}

/// Clean a cpd-finder source id into a readable, openable path: drop a leading `./`,
/// and drop the trailing `:<lang>` tag it appends to name the (fenced-code) language of
/// a clone inside a document — e.g. `docs.md:bash`, `notes.md:markdown`. Without this the
/// finding's path isn't a real file, so it reads oddly *and* the `straitjacket-allow`
/// suppression (which opens the path) silently fails to apply to markdown clones.
fn tidy(path: &str) -> String {
    let path = path.strip_prefix("./").unwrap_or(path);
    if let Some((head, lang)) = path.rsplit_once(':') {
        // Only a genuine language tag: a run of ASCII letters trailing an actual
        // filename. Leaves real path colons (a Windows drive, say) alone.
        let looks_like_path = head.contains('/') || head.contains('.');
        if !lang.is_empty() && lang.bytes().all(|b| b.is_ascii_alphabetic()) && looks_like_path {
            return head.to_string();
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tidy_tests {
    use super::tidy;

    #[test]
    fn strips_leading_dot_slash() {
        assert_eq!(tidy("./src/app.ts"), "src/app.ts");
    }

    #[test]
    fn strips_trailing_language_tag() {
        assert_eq!(tidy("notes/docs.md:bash"), "notes/docs.md");
        assert_eq!(tidy("README.md:markdown"), "README.md");
        assert_eq!(tidy("/abs/path/docs.md:bash"), "/abs/path/docs.md");
    }

    #[test]
    fn leaves_ordinary_paths_untouched() {
        assert_eq!(tidy("src/server/app.ts"), "src/server/app.ts");
        // A Windows drive colon must survive (the tag would follow the filename).
        assert_eq!(tidy(r"C:\proj\docs.md:markdown"), r"C:\proj\docs.md");
        // A bare `word:word` with no path shape isn't treated as a tag.
        assert_eq!(tidy("foo:bar"), "foo:bar");
    }
}
