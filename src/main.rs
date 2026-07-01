use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};

use straitjacket::config::{
    self, FileConfig, DEFAULT_DUP_MIN_TOKENS, DEFAULT_MAX_LINES, DEFAULT_PROSE_WINDOW,
};
use straitjacket::react::{extract_edges, ComponentIndex, REACT_EXTS};
use straitjacket::walk::{collect_files, display_path, ext_of};
use straitjacket::{duplication, prop_graph, Config, Engine, Finding, Severity};

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
}

/// The effective settings after layering CLI flags over the config file over the
/// built-in defaults.
struct Resolved {
    paths: Vec<PathBuf>,
    format: Format,
    only: Vec<String>,
    skip: Vec<String>,
    max_lines: usize,
    prose_window: usize,
    dup_min_tokens: usize,
    include_json: bool,
    no_ignore: bool,
    no_fail: bool,
}

impl Resolved {
    fn respect_ignore(&self) -> bool {
        !self.no_ignore
    }

    fn config(&self) -> Config {
        Config {
            max_lines: (self.max_lines > 0).then_some(self.max_lines),
            slop_prose: true,
            prose_window: self.prose_window,
            duplication: true,
            dup_min_tokens: self.dup_min_tokens,
            skip_json: !self.include_json,
            one_component: true,
            effect_in_component: true,
            prop_drilling: true,
            store_passthrough: true,
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

    if cli.prop_chains {
        return report_prop_chains(&resolved.paths, resolved.respect_ignore());
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

    // The React forwarding rules need a cross-file index of every local component and
    // its prop types — build it from the React files before scanning.
    if engine.needs_component_index() {
        engine.set_component_index(ComponentIndex::build(&react_sources(&files)));
    }

    let mut findings: Vec<Finding> = Vec::new();
    for path in &files {
        let Some(ext) = ext_of(path) else { continue };
        if !engine.handles_ext(&ext) {
            continue;
        }
        // Skip unreadable files and binaries (non-UTF-8) silently.
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        findings.extend(engine.scan_text(&text, &display_path(path), &ext));
    }

    // Cross-file copy/paste detection (compiled in via cpd-finder), run once over
    // the scan paths rather than per file.
    if let Some(min_tokens) = engine.duplication() {
        let ignore: Vec<String> = if engine.skip_json() {
            vec!["**/*.json".to_string()]
        } else {
            Vec::new()
        };
        findings.extend(duplication::detect(
            &resolved.paths,
            resolved.respect_ignore(),
            min_tokens,
            &ignore,
        ));
    }

    report(&resolved.format, &findings, files.len());

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

fn parse_format(s: &str) -> anyhow::Result<Format> {
    match s.trim().to_ascii_lowercase().as_str() {
        "text" => Ok(Format::Text),
        "json" => Ok(Format::Json),
        other => anyhow::bail!("invalid format '{other}' in config (expected text or json)"),
    }
}

fn warn_unknown(unknown: Vec<String>) {
    for id in unknown {
        eprintln!("straitjacket: warning: unknown rule '{id}'");
    }
}

/// Read every React file as `(display_path, source)`.
fn react_sources(files: &[PathBuf]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in files {
        if !ext_of(path).is_some_and(|e| REACT_EXTS.contains(&e.as_str())) {
            continue;
        }
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        out.push((display_path(path), text));
    }
    out
}

/// Collect prop-forwarding edges across all React files, stitch them into chains, and
/// print each chain longest-first with its depth (number of forwarding hops).
fn report_prop_chains(paths: &[PathBuf], respect_ignore: bool) -> ExitCode {
    let sources = react_sources(&collect_files(paths, respect_ignore));
    let index = ComponentIndex::build(&sources);
    let mut edges = Vec::new();
    for (path, text) in &sources {
        edges.extend(extract_edges(text, path));
    }
    // Keep only forwards into a local, non-callback slot — same filter as the rule.
    edges.retain(|e| index.is_drill_target(&e.to_component, &e.to_param));

    let chains = prop_graph::chains(&edges);
    let mut shown = 0;
    for chain in &chains {
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
    if shown == 0 {
        println!("straitjacket: no prop-drilling chains of depth ≥ 2 found");
    } else {
        eprintln!("\nstraitjacket: {shown} prop-drilling chain(s) of depth ≥ 2 (deepest first).");
    }
    ExitCode::SUCCESS
}

fn report(format: &Format, findings: &[Finding], file_count: usize) {
    match format {
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
        }
    }
}
