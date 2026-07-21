use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, ValueEnum};

use straitjacket::config::{
    self, FileConfig, DEFAULT_DUP_MIN_TOKENS, DEFAULT_MAX_LINES, DEFAULT_MAX_NESTING,
    DEFAULT_PROSE_WINDOW,
};
use straitjacket::engine::{collect_markers, suppressed_between, Marker, Suppressed};
use straitjacket::project::Projects;
use straitjacket::react::{extract_edges, ComponentIndex, REACT_EXTS};
use straitjacket::walk::{collect_files, display_path, ext_of};
use straitjacket::{duplication, prop_graph, Config, Engine, Finding, Severity};

/// A scanned file that carries at least one `straitjacket-allow[-file]` marker: what the
/// unused-marker check needs to decide, after every pass, which of its markers did nothing.
struct MarkerFile {
    /// Display path (relative when scanning relative paths) — what the finding reports.
    path: String,
    /// Canonical join key ([`canon_key`]) — how this file is matched against the cross-file
    /// duplication pass, whose findings carry cpd-finder's canonicalized absolute paths. The
    /// two namespaces (relative display vs canonical absolute) only meet through this key.
    key: String,
    ext: String,
    markers: Vec<Marker>,
    /// Would-be violations suppressed in this file — per-file rules first, then any
    /// `duplication` clones dropped in the cross-file pass are appended.
    suppressed: Vec<Suppressed>,
}

/// The canonical join key for a scanned file, matching how `cpd-finder` derives a clone's
/// `source_id`: `fs::canonicalize(path)`, falling back to the raw path when canonicalization
/// fails (identical fallback to cpd-finder's). The cross-file duplication pass reports paths
/// in this canonical-absolute namespace, while the per-file scan keys markers by *display*
/// path; reconciling suppressed clones and wrong-side markers against the right file requires
/// joining on this key, not on the display string. Without it a load-bearing `fragment_a`
/// marker (the one that actually suppresses the clone) is wrongly flagged as unused.
fn canon_key(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Flags weird code and text that LLMs tend to generate, ahead of time.
///
/// Settings can also be checked into a repo as `.straitjacket.yaml`; it's picked
/// up automatically from the current directory or any parent. CLI flags override
/// the file, which overrides the built-in defaults.
#[derive(Parser)]
#[command(name = "straitjacket", version, about, long_about = None)]
struct Cli {
    /// Paths (files or directories) to scan. Defaults to `.` (or the config's paths).
    paths: Vec<PathBuf>,

    /// Output format.
    #[arg(long, value_enum)]
    format: Option<Format>,

    /// Run only these rules (comma-separated ids).
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,

    /// Skip these rules (comma-separated ids).
    #[arg(long, value_delimiter = ',')]
    skip: Vec<String>,

    /// file-size rule's line budget. 0 disables the rule.
    #[arg(long)]
    max_lines: Option<usize>,

    /// deep-nesting rule's indentation-depth budget. 0 disables the rule.
    #[arg(long)]
    max_nesting: Option<usize>,

    /// slop-prose density window, in characters.
    #[arg(long)]
    prose_window: Option<usize>,

    /// duplication rule's minimum clone size, in tokens.
    #[arg(long)]
    dup_min_tokens: Option<usize>,

    /// Also scan .json files (skipped by default as generated/config data).
    #[arg(long)]
    include_json: bool,

    /// Don't respect .gitignore / hidden-file conventions; scan everything.
    #[arg(long)]
    no_ignore: bool,

    /// Exit 0 even when findings exist (report-only).
    #[arg(long)]
    no_fail: bool,

    /// Don't flag `straitjacket-allow[-file]` markers that suppressed nothing (the
    /// unused-marker check is on by default).
    #[arg(long)]
    no_fail_on_unused_markers: bool,

    /// Also write a SARIF 2.1.0 report to this file (alongside the stdout output),
    /// for `github/codeql-action/upload-sarif`.
    #[arg(long, value_name = "PATH")]
    sarif: Option<PathBuf>,

    /// Use this config file instead of discovering `.straitjacket.yaml`.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Ignore any `.straitjacket.yaml`; use only CLI flags and defaults.
    #[arg(long)]
    no_config: bool,

    /// List the available rules and exit.
    #[arg(long)]
    list_rules: bool,

    /// Report cross-file prop-drilling chains (depth analysis) and exit.
    #[arg(long)]
    prop_chains: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
    /// SARIF 2.1.0, for `github/codeql-action/upload-sarif`.
    Sarif,
}

/// The effective settings after layering CLI flags over the config file over the
/// built-in defaults.
struct Resolved {
    paths: Vec<PathBuf>,
    format: Format,
    only: Vec<String>,
    skip: Vec<String>,
    max_lines: usize,
    max_nesting: usize,
    prose_window: usize,
    dup_min_tokens: usize,
    include_json: bool,
    no_ignore: bool,
    no_fail: bool,
    fail_on_unused_markers: bool,
}

impl Resolved {
    fn respect_ignore(&self) -> bool {
        !self.no_ignore
    }

    fn config(&self) -> Config {
        Config {
            max_lines: (self.max_lines > 0).then_some(self.max_lines),
            max_nesting: (self.max_nesting > 0).then_some(self.max_nesting),
            slop_prose: true,
            prose_window: self.prose_window,
            duplication: true,
            dup_min_tokens: self.dup_min_tokens,
            skip_json: !self.include_json,
            one_component: true,
            effect_in_component: true,
            prop_drilling: true,
            store_passthrough: true,
            fail_on_unused_markers: self.fail_on_unused_markers,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let resolved = match resolve(&cli) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("straitjacket: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    // Discover monorepo project boundaries (`.straitjacket.toml` markers). Cross-file
    // analyses partition on these so a clone or forwarding chain is never reported across
    // a package boundary. No markers ⇒ one project ⇒ behaviour identical to before.
    let projects = match Projects::discover(&resolved.paths, resolved.respect_ignore()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("straitjacket: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    if cli.prop_chains {
        return report_prop_chains(&resolved.paths, resolved.respect_ignore(), &projects);
    }

    let mut engine = match Engine::new(&resolved.config()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("straitjacket: failed to build rules: {e}");
            return ExitCode::FAILURE;
        }
    };

    if cli.list_rules {
        for id in engine.rule_ids() {
            println!("{id}\n    {}", engine.message_for(id).unwrap_or_default());
        }
        return ExitCode::SUCCESS;
    }

    if !resolved.only.is_empty() {
        warn_unknown(engine.keep_only(&resolved.only));
    }
    if !resolved.skip.is_empty() {
        warn_unknown(engine.skip(&resolved.skip));
    }

    let files = collect_files(&resolved.paths, resolved.respect_ignore());

    // The React forwarding rules need a cross-file index of every local component and its
    // prop types. Build one index *per project* so a component named the same in two
    // packages (e.g. `Button`) doesn't collide, and a forwarding chain can't span a
    // package boundary. The index for the file being scanned is swapped in as the loop
    // moves between projects (files walk in directory order, so this is a handful of
    // swaps, not one per file).
    let react_indexes = build_react_indexes(&engine, &files, &projects);
    let mut current_project: Option<Option<PathBuf>> = None;

    let mut findings: Vec<Finding> = Vec::new();
    // Files carrying a marker, kept aside so the unused-marker check can run once every
    // pass (including the cross-file duplication one) has had a chance to use each marker.
    let mut marker_files: Vec<MarkerFile> = Vec::new();
    // Canonical join key → display path, over every scanned file, so a wrong-side duplication
    // marker can name `fragment_a` (which the duplication pass reports canonically) with a
    // readable display path — even when that first file carries no marker of its own. Built
    // only when the duplication pass will run.
    let dup_on = engine.duplication().is_some();
    let mut canon_display: HashMap<String, String> = HashMap::new();
    for path in &files {
        let Some(ext) = ext_of(path) else { continue };
        if !engine.handles_ext(&ext) {
            continue;
        }
        if engine.needs_component_index() {
            let key = projects.root_for(path);
            if current_project.as_ref() != Some(&key) {
                let index = react_indexes.get(&key).cloned().unwrap_or_default();
                engine.set_component_index(index);
                current_project = Some(key);
            }
        }
        // Skip unreadable files and binaries (non-UTF-8) silently.
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        let dp = display_path(path);
        // Canonical key joining this file to the cross-file duplication pass (see [`canon_key`]).
        let key = if dup_on { canon_key(path) } else { dp.clone() };
        if dup_on {
            canon_display.insert(key.clone(), dp.clone());
        }
        let visible = engine.scan_text(&text, &dp, &ext);
        // Only files that actually carry a marker can produce an unused-marker finding.
        // Compute the suppressed would-be violations by diffing against the candidate scan.
        // Markdown/MDX prose is exempt: those files carry marker *syntax* as documentation
        // examples and inline notation (`straitjacket-allow[:rule]`), which the check can't
        // tell from a real directive — so the unused-marker check stays on code files.
        if resolved.fail_on_unused_markers
            && !is_doc_ext(&ext)
            && text.contains("straitjacket-allow")
        {
            let markers = collect_markers(&text);
            if !markers.is_empty() {
                let candidates = engine.scan_text_candidates(&text, &dp, &ext);
                let suppressed = suppressed_between(&candidates, &visible);
                marker_files.push(MarkerFile {
                    path: dp.clone(),
                    key: key.clone(),
                    ext: ext.clone(),
                    markers,
                    suppressed,
                });
            }
        }
        findings.extend(visible);
    }

    // Cross-file copy/paste detection (compiled in via cpd-finder). A clone only counts
    // *within* a project, so when boundaries are declared this runs once per project over
    // that project's files — never comparing across a package boundary. With no
    // boundaries it's a single pass over the scan paths, exactly as before.
    // `fragment_b path → fragment_a path` for every clone, so a `duplication` marker sitting
    // on the wrong (second) file of a clone pair can point at where it belongs.
    let mut dup_partners: HashMap<String, String> = HashMap::new();
    let dup_suppressed = if let Some(min_tokens) = engine.duplication() {
        let ignore: Vec<String> = if engine.skip_json() {
            vec!["**/*.json".to_string()]
        } else {
            Vec::new()
        };
        let pairs = duplication::detect_partitioned(
            &resolved.paths,
            &files,
            &projects,
            engine.skip_json(),
            resolved.respect_ignore(),
            min_tokens,
            &ignore,
        );
        // Duplication is a separate cross-file pass (cpd-finder), so its findings don't
        // flow through the per-file `straitjacket-allow` filter — apply the same
        // suppression here so `allow-file:duplication` (and line markers) work for it too.
        // The dropped clones used to vanish uncounted; now they're tallied so a marker
        // masking a pile of clones is visible instead of silently reading as zero.
        let report = duplication::partition(pairs);
        findings.extend(report.kept);
        // A clone dropped by a marker on its home file is a would-be violation that marker
        // suppressed — feed it into the unused-marker reconciliation so the marker reads as
        // used. (Per-file rules were reconciled above; duplication is cross-file.)
        // `report.suppressed` carries `fragment_a`'s *canonical* path, so match on the marker
        // file's canonical key — not its display path, which lives in a different namespace and
        // would never match, leaving the load-bearing marker uncredited and wrongly flagged.
        for (a_path, a_line) in &report.suppressed {
            if let Some(mf) = marker_files.iter_mut().find(|m| &m.key == a_path) {
                mf.suppressed.push(Suppressed {
                    rule: "duplication".to_string(),
                    line: *a_line,
                });
            }
        }
        dup_partners = report.second_file_partner;
        report.tally
    } else {
        duplication::SuppressedTally::default()
    };

    // Now that every pass has run, any marker still credited with nothing is unused.
    if resolved.fail_on_unused_markers {
        for mf in &marker_files {
            // `second_file_partner` is keyed by `fragment_b`'s canonical path → `fragment_a`'s
            // canonical path. Look it up by this file's canonical key, then render the partner as
            // a display path so the "move it to <fragment_a>" message reads like the rest of the
            // output. (Falls back to the canonical path if the first file wasn't scanned.)
            let partner = dup_partners
                .get(&mf.key)
                .map(|a_canon| canon_display.get(a_canon).unwrap_or(a_canon).as_str());
            findings.extend(engine.unused_marker_findings(
                &mf.path,
                &mf.ext,
                &mf.markers,
                &mf.suppressed,
                partner,
            ));
        }
    }

    report(&resolved.format, &findings, files.len(), &dup_suppressed);

    // A side-channel SARIF file, so a run can print readable text *and* hand SARIF to
    // code scanning in one pass.
    if let Some(path) = &cli.sarif {
        let doc = straitjacket::sarif::to_sarif(&findings, env!("CARGO_PKG_VERSION"));
        if let Err(e) = fs::write(path, doc) {
            eprintln!(
                "straitjacket: failed to write SARIF to {}: {e}",
                path.display()
            );
        }
    }

    let has_error = findings.iter().any(|f| f.severity == Severity::Error);
    if has_error && !resolved.no_fail {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Layer CLI flags over a discovered/loaded `.straitjacket.yaml` over the defaults.
fn resolve(cli: &Cli) -> anyhow::Result<Resolved> {
    let file = load_file_config(cli)?;

    let format = match cli.format {
        Some(f) => f,
        None => match file.format.as_deref() {
            Some(s) => parse_format(s)?,
            None => Format::Text,
        },
    };

    let paths = if !cli.paths.is_empty() {
        cli.paths.clone()
    } else if let Some(p) = file.paths {
        p.into_iter().map(PathBuf::from).collect()
    } else {
        vec![PathBuf::from(".")]
    };

    let only = if !cli.only.is_empty() {
        cli.only.clone()
    } else {
        file.only.unwrap_or_default()
    };
    let skip = if !cli.skip.is_empty() {
        cli.skip.clone()
    } else {
        file.skip.unwrap_or_default()
    };

    Ok(Resolved {
        paths,
        format,
        only,
        skip,
        max_lines: cli
            .max_lines
            .or(file.max_lines)
            .unwrap_or(DEFAULT_MAX_LINES),
        max_nesting: cli
            .max_nesting
            .or(file.max_nesting)
            .unwrap_or(DEFAULT_MAX_NESTING),
        prose_window: cli
            .prose_window
            .or(file.prose_window)
            .unwrap_or(DEFAULT_PROSE_WINDOW),
        dup_min_tokens: cli
            .dup_min_tokens
            .or(file.dup_min_tokens)
            .unwrap_or(DEFAULT_DUP_MIN_TOKENS),
        // Booleans: the file sets a baseline; a CLI flag can only turn it on.
        include_json: cli.include_json || file.include_json.unwrap_or(false),
        no_ignore: cli.no_ignore || file.no_ignore.unwrap_or(false),
        no_fail: cli.no_fail || file.no_fail.unwrap_or(false),
        // On by default; the file can turn it off, and the CLI flag can too.
        fail_on_unused_markers: file.fail_on_unused_markers.unwrap_or(true)
            && !cli.no_fail_on_unused_markers,
    })
}

/// Find and load the config file, honoring `--config` and `--no-config`. Prints a
/// one-line note to stderr when a file is used (stdout stays clean for `--format json`).
fn load_file_config(cli: &Cli) -> anyhow::Result<FileConfig> {
    if cli.no_config {
        return Ok(FileConfig::default());
    }
    let path = match &cli.config {
        // An explicit --config path must exist.
        Some(p) => Some(p.clone()),
        None => std::env::current_dir()
            .ok()
            .and_then(|d| config::find_config(&d)),
    };
    match path {
        Some(p) => {
            let cfg = config::load_config(&p)?;
            eprintln!("straitjacket: using config {}", p.display());
            Ok(cfg)
        }
        None => Ok(FileConfig::default()),
    }
}

/// Markdown/MDX documentation extensions, exempt from the unused-marker check because they
/// carry marker syntax as prose examples and notation rather than real directives.
fn is_doc_ext(ext: &str) -> bool {
    matches!(ext, "md" | "markdown" | "mdx")
}

fn parse_format(s: &str) -> anyhow::Result<Format> {
    match s.trim().to_ascii_lowercase().as_str() {
        "text" => Ok(Format::Text),
        "json" => Ok(Format::Json),
        "sarif" => Ok(Format::Sarif),
        other => {
            anyhow::bail!("invalid format '{other}' in config (expected text, json, or sarif)")
        }
    }
}

fn warn_unknown(unknown: Vec<String>) {
    for id in unknown {
        eprintln!("straitjacket: warning: unknown rule '{id}'");
    }
}

/// Group every React file's `(display_path, source)` by the project it belongs to
/// (`None` = the root project). The cross-file React analyses run per bucket.
fn react_sources_by_project(
    files: &[PathBuf],
    projects: &Projects,
) -> HashMap<Option<PathBuf>, Vec<(String, String)>> {
    let mut buckets: HashMap<Option<PathBuf>, Vec<(String, String)>> = HashMap::new();
    for path in files {
        if !ext_of(path).is_some_and(|e| REACT_EXTS.contains(&e.as_str())) {
            continue;
        }
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        buckets
            .entry(projects.root_for(path))
            .or_default()
            .push((display_path(path), text));
    }
    buckets
}

/// Build one `ComponentIndex` per project from the React files, keyed by project root.
/// Empty when the forwarding rules are off (no index needed).
fn build_react_indexes(
    engine: &Engine,
    files: &[PathBuf],
    projects: &Projects,
) -> HashMap<Option<PathBuf>, ComponentIndex> {
    if !engine.needs_component_index() {
        return HashMap::new();
    }
    react_sources_by_project(files, projects)
        .into_iter()
        .map(|(key, sources)| (key, ComponentIndex::build(&sources)))
        .collect()
}

/// Collect prop-forwarding edges across all React files, stitch them into chains, and
/// print each chain longest-first with its depth (number of forwarding hops).
/// A sortable key for a project bucket, so per-project output is deterministic (the root
/// project sorts first as the empty string).
fn bucket_order(key: &Option<PathBuf>) -> String {
    key.as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn report_prop_chains(paths: &[PathBuf], respect_ignore: bool, projects: &Projects) -> ExitCode {
    let files = collect_files(paths, respect_ignore);
    // Chains are stitched *within* each project — an index and edge set per project — so a
    // chain never crosses a package boundary (mirrors how the rules run).
    let mut buckets: Vec<_> = react_sources_by_project(&files, projects)
        .into_iter()
        .collect();
    buckets.sort_by_key(|b| bucket_order(&b.0));

    let mut shown = 0;
    for (_key, sources) in &buckets {
        let index = ComponentIndex::build(sources);
        let mut edges = Vec::new();
        for (path, text) in sources {
            edges.extend(extract_edges(text, path));
        }
        // Keep only forwards into a local, non-callback slot — same filter as the rule.
        edges.retain(|e| index.is_drill_target(&e.to_component, &e.to_param));

        for chain in &prop_graph::chains(&edges) {
            if chain.len() < 2 {
                continue; // depth 1 = a single hop; show real drills (2+)
            }
            shown += 1;
            // Node sequence: first edge's source, then each edge's target.
            let first = &edges[chain[0]];
            let mut nodes = vec![format!("{}.{}", first.from_component, first.from_param)];
            for &ei in chain {
                let e = &edges[ei];
                nodes.push(format!("{}.{}", e.to_component, e.to_param));
            }
            println!("depth {}: {}", chain.len(), nodes.join("  →  "));
            println!("   from {}:{}", first.file, first.line);
        }
    }
    if shown == 0 {
        println!("straitjacket: no prop-drilling chains of depth ≥ 2 found");
    } else {
        eprintln!("\nstraitjacket: {shown} prop-drilling chain(s) of depth ≥ 2 (deepest first).");
    }
    ExitCode::SUCCESS
}

fn report(
    format: &Format,
    findings: &[Finding],
    file_count: usize,
    dup_suppressed: &duplication::SuppressedTally,
) {
    match format {
        Format::Sarif => {
            println!(
                "{}",
                straitjacket::sarif::to_sarif(findings, env!("CARGO_PKG_VERSION"))
            );
        }
        Format::Json => {
            let json = serde_json::to_string_pretty(findings).unwrap_or_else(|_| "[]".to_string());
            println!("{json}");
        }
        Format::Text => {
            for f in findings {
                let tag = match f.severity {
                    Severity::Error => "",
                    Severity::Warning => " (warn)",
                };
                println!(
                    "{}:{}:{}  [{}]{tag}  {}",
                    f.path, f.line, f.col, f.rule, f.matched
                );
            }
            if findings.is_empty() {
                println!("straitjacket: ok — no findings in {file_count} file(s)");
            } else {
                let errors = findings
                    .iter()
                    .filter(|f| f.severity == Severity::Error)
                    .count();
                let warnings = findings.len() - errors;
                eprintln!(
                    "\nstraitjacket: {errors} error(s), {warnings} warning(s) across {file_count} scanned file(s). \
                     Suppress one line with `straitjacket-allow[:rule]`, or a whole \
                     file with `straitjacket-allow-file[:rule]`."
                );
            }
            // An always-on informational tally: clones a marker dropped don't count as
            // findings (they never touch the exit code), but printing how many were
            // masked stops a wall of `allow-file:duplication` markers from reading as a
            // clean zero. Silent when nothing was suppressed.
            if !dup_suppressed.is_empty() {
                eprintln!(
                    "note: duplication: {} clone(s) suppressed by allow-file markers ({} files)",
                    dup_suppressed.clones, dup_suppressed.files
                );
            }
        }
    }
}
