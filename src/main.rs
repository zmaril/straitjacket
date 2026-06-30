use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};

use straitjacket::config::DEFAULT_MAX_LINES;
use straitjacket::walk::{collect_files, display_path, ext_of};
use straitjacket::{Config, Engine, Finding};

/// Flags weird code that LLMs tend to generate, ahead of time.
#[derive(Parser)]
#[command(name = "straitjacket", version, about, long_about = None)]
struct Cli {
    /// Paths (files or directories) to scan.
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Run only these rules (comma-separated ids).
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,

    /// Skip these rules (comma-separated ids).
    #[arg(long, value_delimiter = ',')]
    skip: Vec<String>,

    /// Also scan Markdown files for emoji (off by default).
    #[arg(long)]
    emoji_markdown: bool,

    /// file-size rule's line budget. 0 disables the rule.
    #[arg(long, default_value_t = DEFAULT_MAX_LINES)]
    max_lines: usize,

    /// Don't respect .gitignore / hidden-file conventions; scan everything.
    #[arg(long)]
    no_ignore: bool,

    /// Exit 0 even when findings exist (report-only).
    #[arg(long)]
    no_fail: bool,

    /// List the available rules and exit.
    #[arg(long)]
    list_rules: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let config = Config {
        emoji_in_markdown: cli.emoji_markdown,
        max_lines: (cli.max_lines > 0).then_some(cli.max_lines),
    };

    let mut engine = match Engine::new(&config) {
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

    if !cli.only.is_empty() {
        warn_unknown(engine.keep_only(&cli.only));
    }
    if !cli.skip.is_empty() {
        warn_unknown(engine.skip(&cli.skip));
    }

    let files = collect_files(&cli.paths, !cli.no_ignore);
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

    report(&cli.format, &findings, files.len());

    if findings.is_empty() || cli.no_fail {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn warn_unknown(unknown: Vec<String>) {
    for id in unknown {
        eprintln!("straitjacket: warning: unknown rule '{id}'");
    }
}

fn report(format: &Format, findings: &[Finding], file_count: usize) {
    match format {
        Format::Json => {
            let json = serde_json::to_string_pretty(findings).unwrap_or_else(|_| "[]".to_string());
            println!("{json}");
        }
        Format::Text => {
            for f in findings {
                println!(
                    "{}:{}:{}  [{}]  {}",
                    f.path, f.line, f.col, f.rule, f.matched
                );
            }
            if findings.is_empty() {
                println!("straitjacket: ok — no findings in {file_count} file(s)");
            } else {
                eprintln!(
                    "\nstraitjacket: {} finding(s) across {} scanned file(s). \
                     Suppress one line with `straitjacket-allow[:rule]`, or a whole \
                     file with `straitjacket-allow-file[:rule]`.",
                    findings.len(),
                    file_count
                );
            }
        }
    }
}
