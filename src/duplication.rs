//! `duplication` — copy/paste detection compiled directly into straitjacket via the
//! `cpd-finder` library (the engine behind jscpd 5, which is itself Rust). No
//! external binary to install and no Node: straitjacket walks and tokenizes the
//! tree itself and reports every clone.
//!
//! The policy matches straitjacket's max-by-default stance: **a structure may appear
//! only once.** Any clone of at least `min_tokens` tokens is an `Error`. This is a
//! cross-file, whole-run analysis, so it runs once over the scan paths rather than
//! per file.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

use cpd_finder::orchestrate::{run, RunConfig};

use crate::engine::is_suppressed;
use crate::finding::{Finding, Severity};
use crate::project::Projects;
use crate::walk::ext_of;

const RULE: &str = "duplication";

/// How many duplication clone-pairs a run dropped because a `straitjacket-allow`
/// / `straitjacket-allow-file` marker covered them, plus how many distinct files
/// carried such a marker. Emitted as an informational note — it never affects the
/// exit code — so a masked pile of clones stops being invisible in CI.
#[derive(Debug, Default)]
pub struct SuppressedTally {
    /// Suppressed clone-pairs (one per dropped duplication finding).
    pub clones: usize,
    /// Distinct files carrying a marker that suppressed at least one clone.
    pub files: usize,
}

impl SuppressedTally {
    /// True when nothing was suppressed, so callers can stay silent.
    pub fn is_empty(&self) -> bool {
        self.clones == 0
    }
}

/// One detected clone pair: the finding straitjacket reports (anchored at `fragment_a`, the
/// alphabetically-first file, exactly as before) plus the *second* file's path and line.
/// The second file is retained because suppression only ever reads `fragment_a`, so a marker
/// on the second file is dead by construction — the `unused-marker` check needs to see it to
/// explain that.
#[derive(Debug, Clone)]
pub struct ClonePair {
    /// The reported finding, anchored at `fragment_a`.
    pub finding: Finding,
    /// The `fragment_b` (alphabetically-later) file's display path.
    pub b_path: String,
    /// The clone's 1-based start line in `fragment_b`.
    pub b_line: usize,
}

/// The outcome of reconciling a run's clone pairs against their `straitjacket-allow[-file]`
/// markers: what survives, an informational tally of what a marker dropped, the would-be
/// duplication violations that were suppressed (for the unused-marker reconciliation), and a
/// `second-file → first-file` map so a dead marker on the wrong side of a clone can point at
/// where it belongs.
#[derive(Debug, Default)]
pub struct DupReport {
    /// Clones that survived suppression — the reported findings.
    pub kept: Vec<Finding>,
    /// How many clones a marker dropped, and across how many files.
    pub tally: SuppressedTally,
    /// `(fragment_a path, fragment_a line)` for every clone a marker on its home file dropped.
    pub suppressed: Vec<(String, usize)>,
    /// `fragment_b path → fragment_a path` over every clone (dead-marker side → live side).
    pub second_file_partner: HashMap<String, String>,
}

/// Reconcile clone pairs against their markers. A clone is suppressed exactly as the old
/// filter decided: read its home file (`fragment_a`) and ask [`is_suppressed`]. An unreadable
/// file is treated as *not* suppressed (the finding is kept), matching the previous
/// `unwrap_or(true)` keep-predicate.
pub fn partition(pairs: Vec<ClonePair>) -> DupReport {
    let mut report = DupReport::default();
    let mut suppressed_files: HashSet<String> = HashSet::new();
    for pair in pairs {
        // Record the wrong-side mapping for every clone (keep the smallest home path when a
        // second file belongs to several clones, so the message is deterministic).
        report
            .second_file_partner
            .entry(pair.b_path.clone())
            .and_modify(|a| {
                if pair.finding.path < *a {
                    *a = pair.finding.path.clone();
                }
            })
            .or_insert_with(|| pair.finding.path.clone());

        let f = pair.finding;
        let suppressed = fs::read_to_string(&f.path)
            .map(|text| is_suppressed(&text, f.line, &f.rule))
            .unwrap_or(false);
        if suppressed {
            report.tally.clones += 1;
            suppressed_files.insert(f.path.clone());
            report.suppressed.push((f.path.clone(), f.line));
        } else {
            report.kept.push(f);
        }
    }
    report.tally.files = suppressed_files.len();
    report
}

/// Backwards-compatible split into surviving findings and the suppressed tally. A thin
/// wrapper over [`partition`] for callers that don't need the wrong-side detail.
pub fn partition_suppressed(pairs: Vec<ClonePair>) -> (Vec<Finding>, SuppressedTally) {
    let report = partition(pairs);
    (report.kept, report.tally)
}

/// Run copy/paste detection, partitioning by project when boundaries are declared.
///
/// With no boundaries it's a single pass over `scan_paths` (original behaviour). With
/// boundaries it runs one cpd pass per project over that project's explicit file list, so
/// a clone is only ever reported *within* a project — two independent packages that share
/// boilerplate aren't flagged. Per-project file lists (not one global pass then filtered)
/// are used on purpose: cpd reports only a subset of clone pairs, so post-filtering could
/// drop a genuine in-project clone reported only via a cross-project pairing.
///
/// `files` is the already-collected scan set; `skip_json` drops `.json` from the
/// per-project passes (duplication never reads it when JSON is skipped).
pub fn detect_partitioned(
    scan_paths: &[PathBuf],
    files: &[PathBuf],
    projects: &Projects,
    skip_json: bool,
    respect_ignore: bool,
    min_tokens: usize,
    ignore: &[String],
) -> Vec<ClonePair> {
    if !projects.is_partitioned() {
        return detect(scan_paths, respect_ignore, min_tokens, ignore);
    }
    let mut buckets: HashMap<Option<PathBuf>, Vec<PathBuf>> = HashMap::new();
    for path in files {
        if skip_json && ext_of(path).as_deref() == Some("json") {
            continue;
        }
        buckets
            .entry(projects.root_for(path))
            .or_default()
            .push(path.clone());
    }
    // Deterministic order (root project first as the empty key) so output is stable.
    let mut ordered: Vec<_> = buckets.into_iter().collect();
    ordered.sort_by_key(|a| bucket_key(&a.0));
    let mut out = Vec::new();
    for (_key, bucket) in &ordered {
        out.extend(detect(bucket, respect_ignore, min_tokens, ignore));
    }
    out
}

/// A sortable key for a project bucket; the root project (`None`) sorts first.
fn bucket_key(key: &Option<PathBuf>) -> String {
    key.as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Detect duplicated code across `paths` and return one finding per clone. `paths` may
/// be directories or individual files; passing one project's files at a time is how the
/// caller keeps clones from being reported across a monorepo boundary. `ignore` holds
/// extra glob patterns to exclude (e.g. `**/*.json`). Detection failures degrade to an
/// empty result rather than aborting the whole scan.
pub fn detect(
    paths: &[PathBuf],
    respect_ignore: bool,
    min_tokens: usize,
    ignore: &[String],
) -> Vec<ClonePair> {
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
            let b_path = tidy(&b.source_id);
            let b_line = b.start.line as usize;
            let finding = Finding {
                rule: RULE.to_string(),
                path: tidy(&a.source_id),
                line: a.start.line as usize,
                col: a.start.column as usize,
                matched: format!("{lines} lines, {} tokens", clone.token_count),
                message: format!(
                    "duplicated code — this block also appears at {b_path}:{b_line}. LLMs clone-and-tweak; factor out a shared helper.",
                ),
                severity: Severity::Error,
            };
            ClonePair {
                finding,
                b_path,
                b_line,
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
