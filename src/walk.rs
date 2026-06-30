//! File discovery. Uses the `ignore` crate (ripgrep's walker) so `.gitignore`,
//! `.ignore`, and hidden-file conventions are respected by default — the scan sees
//! the same files a developer thinks of as "the repo".

use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

/// Collect candidate files under `roots`. When `respect_ignore` is false, ignore
/// files and hidden-file filtering are turned off (scan everything).
pub fn collect_files(roots: &[PathBuf], respect_ignore: bool) -> Vec<PathBuf> {
    let mut roots = roots.iter();
    let Some(first) = roots.next() else {
        return Vec::new();
    };

    let mut builder = WalkBuilder::new(first);
    for r in roots {
        builder.add(r);
    }
    builder
        .hidden(respect_ignore)
        .git_ignore(respect_ignore)
        .git_global(respect_ignore)
        .git_exclude(respect_ignore)
        .ignore(respect_ignore)
        .parents(respect_ignore);

    let mut files = Vec::new();
    for result in builder.build() {
        let Ok(entry) = result else { continue };
        if entry.file_type().is_some_and(|t| t.is_file()) {
            files.push(entry.into_path());
        }
    }
    files
}

/// Lowercased extension without the dot, e.g. `Some("tsx")`. `None` for files
/// without an extension.
pub fn ext_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// Tidy a path for display: drop a leading `./`.
pub fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.strip_prefix("./").unwrap_or(&s).to_string()
}
