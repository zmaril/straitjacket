//! Behavioural tests for the built-in rules, driven through the real `Engine` the
//! same way the CLI uses it. Each case feeds a snippet to `scan_text` and asserts on
//! the findings, covering the slop signals, the `straitjacket-allow` escape hatch,
//! per-rule extension scoping, and the false-positive boundaries that separate real
//! emoji / colors / fonts from look-alikes.

use straitjacket::{Config, Engine};

fn engine() -> Engine {
    Engine::new(&Config::default()).expect("rules compile")
}

fn engine_with(config: Config) -> Engine {
    Engine::new(&config).expect("rules compile")
}

/// Scan a snippet for a given extension and return the rule ids that fired, in
/// order, paired with the matched text.
fn scan(src: &str, ext: &str) -> Vec<(String, String)> {
    engine()
        .scan_text(src, "test", ext)
        .into_iter()
        .map(|f| (f.rule, f.matched))
        .collect()
}

fn rules_hit(src: &str, ext: &str) -> Vec<String> {
    scan(src, ext).into_iter().map(|(r, _)| r).collect()
}

// ---- emoji -------------------------------------------------------------------

// Emoji in these fixtures are written as `\u{...}` escapes on purpose: it keeps the
// test source itself emoji-free so straitjacket's own dogfood self-scan stays clean,
// while the strings the rule sees still contain the real glyphs.

#[test]
fn flags_color_emoji() {
    assert_eq!(rules_hit("const ok = '\u{1F680}';", "ts"), vec!["emoji"]); // rocket
    assert_eq!(rules_hit("// done \u{2705}", "ts"), vec!["emoji"]); // check mark button
}

#[test]
fn flags_vs16_presented_glyph() {
    // U+2702 (scissors) is text-default, but the trailing VS16 makes it an emoji.
    assert_eq!(
        rules_hit("label('\u{2702}\u{FE0F} cut')", "ts"),
        vec!["emoji"]
    );
}

#[test]
fn emoji_reports_glyph_with_its_vs16() {
    let hits = scan("x = '\u{25B6}\u{FE0F}'", "ts"); // play button
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "emoji");
    assert_eq!(hits[0].1, "\u{25B6}\u{FE0F}");
}

#[test]
fn allows_plain_typography_and_text_symbols() {
    // Arrows, dashes, ellipsis, curly quotes, the geometric star, and ©/™ are not
    // emoji and must not be flagged.
    let benign = "// a → b — c … \u{201C}q\u{201D} ★ © ™ ® · 42";
    assert!(
        rules_hit(benign, "ts").is_empty(),
        "got {:?}",
        rules_hit(benign, "ts")
    );
}

#[test]
fn emoji_not_scanned_in_markdown_by_default() {
    // Markdown emoji is opt-in, so a default scan leaves it alone.
    assert!(rules_hit("# Hi \u{1F680}", "md").is_empty());
}

#[test]
fn emoji_scanned_in_markdown_when_enabled() {
    let e = engine_with(Config {
        emoji_in_markdown: true,
        ..Config::default()
    });
    let hits: Vec<String> = e
        .scan_text("# Hi \u{1F680}", "README.md", "md")
        .into_iter()
        .map(|f| f.rule)
        .collect();
    assert_eq!(hits, vec!["emoji"]);
}

// ---- file-size ---------------------------------------------------------------

#[test]
fn flags_files_over_the_line_budget() {
    let e = engine_with(Config {
        max_lines: Some(3),
        ..Config::default()
    });
    let src = "a\nb\nc\nd\ne\n";
    let findings = e.scan_text(src, "big.ts", "ts");
    let size: Vec<_> = findings.iter().filter(|f| f.rule == "file-size").collect();
    assert_eq!(size.len(), 1);
    assert_eq!(size[0].line, 4); // first line past the 3-line budget
    assert_eq!(size[0].matched, "5 lines");
}

#[test]
fn file_at_or_under_budget_is_fine() {
    let e = engine_with(Config {
        max_lines: Some(3),
        ..Config::default()
    });
    assert!(e.scan_text("a\nb\nc\n", "ok.ts", "ts").is_empty());
}

#[test]
fn file_size_disabled_when_max_lines_none() {
    let e = engine_with(Config {
        max_lines: None,
        ..Config::default()
    });
    assert!(e.scan_text("a\nb\nc\nd\n", "big.ts", "ts").is_empty());
}

#[test]
fn file_size_exempted_by_file_marker() {
    let e = engine_with(Config {
        max_lines: Some(2),
        ..Config::default()
    });
    // An explicit `-file:file-size` marker anywhere in the file exempts it.
    let src = "a\nb\nc\nd // straitjacket-allow-file:file-size generated\n";
    assert!(e.scan_text(src, "gen.ts", "ts").is_empty());
    // A *line-scoped* allow does NOT silence the whole-file check.
    let bare = "a // straitjacket-allow\nb\nc\nd\n";
    let hit: Vec<_> = e
        .scan_text(bare, "gen.ts", "ts")
        .into_iter()
        .filter(|f| f.rule == "file-size")
        .collect();
    assert_eq!(hit.len(), 1);
}

// ---- whole-file allow --------------------------------------------------------

#[test]
fn file_marker_exempts_a_rule_for_the_whole_file() {
    // A palette file: one marker up top exempts hex-color everywhere in the file...
    let src = "/* straitjacket-allow-file:hex-color palette */\n\
               --a: #111;\n--b: #222;\n--c: #333;\n";
    assert!(rules_hit(src, "css").is_empty());
}

#[test]
fn file_marker_is_rule_scoped() {
    // ...but only the named rule — other rules still fire elsewhere in the file.
    let src = "/* straitjacket-allow-file:hex-color */\n\
               --a: #111;\nfont-family: Inter, sans-serif;\n";
    assert_eq!(rules_hit(src, "css"), vec!["inline-font"]);
}

#[test]
fn bare_file_marker_exempts_all_rules() {
    let src = "/* straitjacket-allow-file generated */\n\
               --a: #111;\nfont-family: Inter;\n";
    assert!(rules_hit(src, "css").is_empty());
}

#[test]
fn file_size_can_be_skipped() {
    let mut e = engine_with(Config {
        max_lines: Some(2),
        ..Config::default()
    });
    e.skip(&["file-size".to_string()]);
    assert!(e.scan_text("a\nb\nc\nd\n", "big.ts", "ts").is_empty());
}

// ---- hex-color ---------------------------------------------------------------

#[test]
fn flags_hex_colors() {
    assert_eq!(
        scan("color: #1e1e1e;", "css"),
        vec![("hex-color".into(), "#1e1e1e".into())]
    );
    assert_eq!(rules_hit("background:#fff", "css"), vec!["hex-color"]);
    assert_eq!(rules_hit("const c = '#aabbccdd'", "tsx"), vec!["hex-color"]);
}

#[test]
fn hex_color_ignores_non_color_hex_runs() {
    // A 5- or 7-digit run isn't a valid hex color length.
    assert!(rules_hit("id = #12345;", "css").is_empty());
}

// ---- inline-svg --------------------------------------------------------------

#[test]
fn flags_inline_svg() {
    assert_eq!(
        rules_hit("return <svg viewBox='0 0 1 1' />;", "tsx"),
        vec!["inline-svg"]
    );
    assert_eq!(
        rules_hit(r#"createElement("svg", {})"#, "ts"),
        vec!["inline-svg"]
    );
}

#[test]
fn inline_svg_only_in_component_sources() {
    // Plain HTML/CSS aren't component code; <svg there isn't the smell.
    assert!(rules_hit("<svg></svg>", "html").is_empty());
}

// ---- inline-font -------------------------------------------------------------

#[test]
fn flags_inline_font_stack() {
    assert_eq!(
        scan("font-family: Inter, sans-serif;", "css"),
        vec![("inline-font".into(), "Inter, sans-serif".into())]
    );
    assert_eq!(
        rules_hit("style={{ fontFamily: 'ui-monospace' }}", "tsx"),
        vec!["inline-font"]
    );
}

#[test]
fn inline_font_allows_variables_and_keywords() {
    assert!(rules_hit("font-family: var(--app-font);", "css").is_empty());
    assert!(rules_hit("font-family: inherit;", "css").is_empty());
}

// ---- motion ------------------------------------------------------------------

#[test]
fn flags_ad_hoc_motion() {
    assert_eq!(
        rules_hit("transition: all 0.2s ease;", "css"),
        vec!["motion"]
    );
    assert_eq!(rules_hit("animation-name: spin;", "css"), vec!["motion"]);
    assert_eq!(rules_hit("@keyframes spin { }", "css"), vec!["motion"]);
}

#[test]
fn motion_ignores_non_declaration_uses() {
    // `transitionProps` (a component prop) and the bare word in prose aren't a `:`
    // declaration, so they don't match.
    assert!(rules_hit("<Modal transitionProps={{}} />", "tsx").is_empty());
    assert!(rules_hit("// the transition between states", "ts").is_empty());
}

// ---- escape hatch ------------------------------------------------------------

#[test]
fn bare_allow_suppresses_every_rule_on_the_line() {
    assert!(rules_hit("color: #fff; // straitjacket-allow: brand chrome", "css").is_empty());
}

#[test]
fn scoped_allow_suppresses_only_that_rule() {
    // Suppress hex but not the emoji on the same line.
    let src = "x = '#fff' // \u{1F680} straitjacket-allow:hex-color";
    assert_eq!(rules_hit(src, "ts"), vec!["emoji"]);
}

// ---- selection ---------------------------------------------------------------

#[test]
fn only_runs_selected_rules() {
    let mut e = engine();
    let unknown = e.keep_only(&["emoji".to_string()]);
    assert!(unknown.is_empty());
    // hex disabled, no emoji on the line → nothing fires.
    let hits: Vec<String> = e
        .scan_text("color: #fff;", "css", "css")
        .into_iter()
        .map(|f| f.rule)
        .collect();
    assert!(hits.is_empty(), "hex disabled, no emoji — got {hits:?}");
    let ts: Vec<String> = e
        .scan_text("const x = '\u{1F680}'", "test", "ts")
        .into_iter()
        .map(|f| f.rule)
        .collect();
    assert_eq!(ts, vec!["emoji"]);
}

#[test]
fn skip_disables_a_rule_and_reports_unknown() {
    let mut e = engine();
    let unknown = e.skip(&["hex-color".to_string(), "nope".to_string()]);
    assert_eq!(unknown, vec!["nope"]);
    assert!(e.scan_text("color: #fff;", "test", "css").is_empty());
}

#[test]
fn positions_are_one_based() {
    let f = &engine().scan_text("  color: #fff;", "f.css", "css")[0];
    assert_eq!(f.line, 1);
    assert_eq!(f.col, 10); // '#' is the 10th byte (1-based)
}
