//! Behavioural tests for the built-in rules, driven through the real `Engine` the
//! same way the CLI uses it. Each case feeds a snippet to `scan_text` and asserts on
//! the findings, covering the slop signals, the `straitjacket-allow` escape hatch,
//! per-rule extension scoping, and the false-positive boundaries that separate real
//! emoji / colors / fonts from look-alikes.

use straitjacket::{Config, Engine, Severity};

fn engine() -> Engine {
    Engine::new(&Config::default()).expect("rules compile")
}

fn prose_engine() -> Engine {
    engine_with(Config {
        slop_prose: true,
        ..Config::default()
    })
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

/// Rule ids that fired for a snippet under a specific (non-default) engine.
fn rules_hit_with(e: &Engine, src: &str, ext: &str) -> Vec<String> {
    e.scan_text(src, "test", ext)
        .into_iter()
        .map(|f| f.rule)
        .collect()
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
fn emoji_scanned_in_markdown_by_default() {
    // Markdown is scanned for emoji by default — docs are where they're least welcome.
    assert_eq!(rules_hit("# Hi \u{1F680}", "md"), vec!["emoji"]);
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
    // A palette file: one marker up top exempts color everywhere in the file...
    let src = "/* straitjacket-allow-file:color palette */\n\
               --a: #111;\n--b: #222;\n--c: #333;\n";
    assert!(rules_hit(src, "css").is_empty());
}

#[test]
fn file_marker_is_rule_scoped() {
    // ...but only the named rule — other rules still fire elsewhere in the file.
    let src = "/* straitjacket-allow-file:color */\n\
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

// ---- deep-nesting ------------------------------------------------------------

fn nest_engine() -> Engine {
    engine_with(Config {
        max_nesting: Some(2),
        ..Config::default()
    })
}

#[test]
fn flags_indentation_past_the_budget() {
    // 2-space unit; the depth-3 line is over a budget of 2.
    let src = "fn f() {\n  a\n    b\n      c\n}\n";
    let f: Vec<_> = nest_engine()
        .scan_text(src, "deep.rs", "rs")
        .into_iter()
        .filter(|f| f.rule == "deep-nesting")
        .collect();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].line, 4);
    assert_eq!(f[0].matched, "nesting depth 3");
    assert_eq!(f[0].severity, Severity::Warning); // a warn, not a hard fail
}

#[test]
fn nesting_within_budget_is_clean() {
    let src = "fn f() {\n  a\n    b\n}\n";
    assert!(!rules_hit_with(&nest_engine(), src, "rs").contains(&"deep-nesting".to_string()));
}

#[test]
fn nesting_disabled_when_max_nesting_none() {
    let e = engine_with(Config {
        max_nesting: None,
        ..Config::default()
    });
    let src = "fn f() {\n  a\n    b\n      c\n        d\n}\n";
    assert!(!rules_hit_with(&e, src, "rs").contains(&"deep-nesting".to_string()));
}

#[test]
fn nesting_ignores_markup_and_data_extensions() {
    // Deeply-indented YAML/Markdown nest by nature — not a code smell.
    let src = "a:\n  b:\n    c:\n      d:\n        e: 1\n";
    assert!(nest_engine().scan_text(src, "deep.yaml", "yaml").is_empty());
}

#[test]
fn nesting_line_marker_silences_the_offending_line() {
    let src = "fn f() {\n  a\n    b\n      c // straitjacket-allow:deep-nesting\n}\n";
    assert!(!rules_hit_with(&nest_engine(), src, "rs").contains(&"deep-nesting".to_string()));
}

#[test]
fn nesting_file_marker_exempts_the_whole_file() {
    let src = "// straitjacket-allow-file:deep-nesting generated\n\
               fn f() {\n  a\n    b\n      c\n}\n";
    assert!(!rules_hit_with(&nest_engine(), src, "rs").contains(&"deep-nesting".to_string()));
}

#[test]
fn nesting_can_be_skipped() {
    let mut e = nest_engine();
    e.skip(&["deep-nesting".to_string()]);
    let src = "fn f() {\n  a\n    b\n      c\n}\n";
    assert!(e.scan_text(src, "deep.rs", "rs").is_empty());
}

// ---- color -------------------------------------------------------------------

#[test]
fn flags_hex_colors() {
    assert_eq!(
        scan("color: #1e1e1e;", "css"),
        vec![("color".into(), "#1e1e1e".into())]
    );
    assert_eq!(rules_hit("background:#fff", "css"), vec!["color"]);
    assert_eq!(rules_hit("const c = '#aabbccdd'", "tsx"), vec!["color"]);
}

#[test]
fn flags_functional_colors() {
    // rgb/rgba/hsl and modern spaces are the same "hardcoded color" smell.
    assert_eq!(
        scan("color: rgb(255, 0, 0);", "css"),
        vec![("color".into(), "rgb(255, 0, 0)".into())]
    );
    assert_eq!(
        rules_hit("background: rgba(0,0,0,.5);", "css"),
        vec!["color"]
    );
    assert_eq!(
        rules_hit("border: 1px solid hsl(210 40% 96%);", "css"),
        vec!["color"]
    );
    assert_eq!(rules_hit("--x: oklch(0.7 0.1 200);", "css"), vec!["color"]);
    // The color() function, but only with a real color space.
    assert_eq!(
        rules_hit("--x: color(display-p3 1 0 0);", "css"),
        vec!["color"]
    );
}

#[test]
fn color_ignores_non_color_hex_runs() {
    // A 5- or 7-digit run isn't a valid hex color length.
    assert!(rules_hit("id = #12345;", "css").is_empty());
}

#[test]
fn color_ignores_pascalcase_constructors() {
    // `Color(...)` / `Rgb(...)` in code aren't lowercase CSS color functions.
    assert!(rules_hit("const c = new Color(1, 2, 3);", "ts").is_empty());
}

#[test]
fn color_ignores_english_color_word() {
    // "color(s)" / "color(ful)" are prose, not the CSS color() function.
    assert!(rules_hit("// handles the color(s) of the theme", "ts").is_empty());
    assert!(rules_hit("a colorful color(ful) note", "ts").is_empty());
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

#[test]
fn inline_font_allows_a_token_reference() {
    // A font token / variable reference is the *good* pattern, not a literal stack —
    // including in a JS object where the trailing comma separates properties.
    assert!(rules_hit("const s = { fontFamily: MONO };", "tsx").is_empty());
    assert!(rules_hit("const s = { fontFamily: SANS, fontSize: 12 };", "tsx").is_empty());
}

#[test]
fn inline_font_allows_a_quoted_css_var() {
    // A CSS variable is a token reference whether bare or quoted — Mantine exposes its
    // font tokens as `var(--mantine-font-family-monospace)`, often quoted in JS style.
    assert!(rules_hit(
        r#"style={{ fontFamily: "var(--mantine-font-family-monospace)" }}"#,
        "tsx"
    )
    .is_empty());
    assert!(rules_hit("font-family: 'var(--app-font)';", "css").is_empty());
    // ...but a quoted *font* is still a hardcoded literal, so it stays flagged.
    assert_eq!(
        rules_hit(r#"style={{ fontFamily: "Inter" }}"#, "tsx"),
        vec!["inline-font"]
    );
}

#[test]
fn inline_font_allows_a_bare_generic_family() {
    // A lone generic family (or a single bare word) isn't a hardcoded stack.
    assert!(rules_hit("font-family: monospace;", "css").is_empty());
    assert!(rules_hit("font-family: sans-serif;", "css").is_empty());
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
    let src = "x = '#fff' // \u{1F680} straitjacket-allow:color";
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
    let unknown = e.skip(&["color".to_string(), "nope".to_string()]);
    assert_eq!(unknown, vec!["nope"]);
    assert!(e.scan_text("color: #fff;", "test", "css").is_empty());
}

#[test]
fn positions_are_one_based() {
    let f = &engine().scan_text("  color: #fff;", "f.css", "css")[0];
    assert_eq!(f.line, 1);
    assert_eq!(f.col, 10); // '#' is the 10th byte (1-based)
}

// ---- slop-prose --------------------------------------------------------------

fn prose(src: &str) -> Vec<straitjacket::Finding> {
    prose_engine().scan_text(src, "doc.md", "md")
}

#[test]
fn slop_prose_is_on_by_default() {
    // straitjacket runs at its max: the default engine catches slop-prose too.
    let f = engine().scan_text("As an AI language model, I cannot.", "doc.md", "md");
    assert!(f.iter().any(|f| f.rule == "slop-prose"));
}

#[test]
fn slop_prose_scans_html() {
    let f = prose_engine().scan_text("<p>utm_source=chatgpt.com</p>", "page.html", "html");
    assert!(f.iter().any(|f| f.rule == "slop-prose"));
}

#[test]
fn slop_prose_tier0_artifact_hard_fails() {
    let f = prose("See the source utm_source=chatgpt.com here.");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].rule, "slop-prose");
    assert_eq!(f[0].severity, Severity::Error);
    assert_eq!(f[0].matched, "utm_source=chatgpt.com");
}

#[test]
fn slop_prose_dense_style_is_an_error() {
    let src = "It stands as a testament to the rich cultural heritage, showcasing a vibrant \
               and intricate tapestry that underscores its pivotal, meticulous role.";
    let f = prose(src);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].rule, "slop-prose");
    assert_eq!(f[0].severity, Severity::Error);
}

#[test]
fn slop_prose_moderate_density_only_warns() {
    let src = "The museum boasts a vibrant collection and plays a vital role in the town.";
    let f = prose(src);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].severity, Severity::Warning);
}

#[test]
fn slop_prose_ignores_ordinary_prose() {
    let src = "Install the dependencies and run the tests. The build produces a single \
               binary you can drop into CI.";
    assert!(prose(src).is_empty());
}

#[test]
fn slop_prose_short_text_does_not_blow_up() {
    // Dividing by the fixed window keeps a lone template from failing tiny text.
    assert!(prose("not X, but Y").is_empty());
}

#[test]
fn slop_prose_only_scans_prose_extensions() {
    // The same artifact in a .ts file is not slop-prose's concern.
    let f = prose_engine().scan_text("x = 'utm_source=chatgpt.com'", "a.ts", "ts");
    assert!(f.iter().all(|f| f.rule != "slop-prose"));
}

#[test]
fn slop_prose_file_marker_exempts_the_whole_file() {
    let src = "<!-- straitjacket-allow-file:slop-prose -->\nAs an AI language model, I cannot.";
    assert!(prose(src).is_empty());
}

#[test]
fn slop_prose_line_marker_skips_an_artifact() {
    let src = "utm_source=chatgpt.com <!-- straitjacket-allow -->";
    assert!(prose(src).is_empty());
}

#[test]
fn slop_prose_can_be_skipped() {
    let mut e = prose_engine();
    e.skip(&["slop-prose".to_string()]);
    assert!(e
        .scan_text("As an AI language model, I cannot.", "doc.md", "md")
        .is_empty());
}

// ---- duplication -------------------------------------------------------------

/// Write files into a fresh temp dir, run duplication over it, clean up, return the
/// findings. `files` is (name, contents) pairs.
fn detect_dups(
    tag: &str,
    files: &[(&str, String)],
    min_tokens: usize,
) -> Vec<straitjacket::Finding> {
    use std::fs;
    let dir = std::env::temp_dir().join(format!("sj-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (name, contents) in files {
        fs::write(dir.join(name), contents).unwrap();
    }
    let findings =
        straitjacket::duplication::detect(std::slice::from_ref(&dir), true, min_tokens, &[]);
    let _ = fs::remove_dir_all(&dir);
    findings
}

fn func(name: &str) -> String {
    format!("fn {name}() {{\n let a = 1;\n let b = 2;\n let c = 3;\n let d = 4;\n let e = 5;\n println!(\"{{}}\", a + b + c + d + e);\n}}\n")
}

#[test]
fn duplication_flags_a_clone() {
    let src = format!("{}{}", func("alpha"), func("beta"));
    let findings = detect_dups("dup", &[("d.rs", src)], 20);
    assert!(
        findings
            .iter()
            .any(|f| f.rule == "duplication" && f.severity == Severity::Error),
        "expected a duplication error, got {findings:?}"
    );
}

#[test]
fn duplication_ignores_unique_code() {
    let src = "fn one() { println!(\"hello world\"); }\n\
               fn two() { let mut t = 0; for i in 0..9 { t += i * 7 - 1; } dbg!(t); }\n"
        .to_string();
    assert!(detect_dups("nodup", &[("u.rs", src)], 20).is_empty());
}

// ---- json skipping -----------------------------------------------------------

#[test]
fn json_is_skipped_by_default() {
    // JSON is generated/config data — not scanned at all by default.
    assert!(!engine().handles_ext("json"));
    let big = "{}\n".repeat(3000);
    assert!(engine().scan_text(&big, "data.json", "json").is_empty());
}

#[test]
fn json_scanned_when_included() {
    let e = engine_with(Config {
        skip_json: false,
        max_lines: Some(10),
        ..Config::default()
    });
    assert!(e.handles_ext("json"));
    let big = "{}\n".repeat(20);
    let f = e.scan_text(&big, "data.json", "json");
    assert!(f.iter().any(|f| f.rule == "file-size"));
}

// ---- duplication (cont.) -----------------------------------------------------

// ---- react (OXC AST) ---------------------------------------------------------

/// Scan `src`, building the cross-file component index from `src` plus `others`
/// (extra files defining the child components a forward targets).
fn tsx_with(src: &str, others: &[&str]) -> Vec<String> {
    use straitjacket::react::ComponentIndex;
    let mut sources = vec![("Main.tsx".to_string(), src.to_string())];
    for (i, o) in others.iter().enumerate() {
        sources.push((format!("Other{i}.tsx"), o.to_string()));
    }
    let mut e = engine();
    e.set_component_index(ComponentIndex::build(&sources));
    e.scan_text(src, "Main.tsx", "tsx")
        .into_iter()
        .map(|f| f.rule)
        .collect()
}

fn tsx(src: &str) -> Vec<String> {
    tsx_with(src, &[])
}

#[test]
fn one_component_flags_a_second_component() {
    let src = "export function Foo() { return <div/>; }\nexport const Bar = () => <span/>;\n";
    assert!(tsx(src).contains(&"one-component".to_string()));
}

#[test]
fn one_component_ok_with_a_single_component() {
    assert!(tsx("export function Foo() { return <div/>; }\n").is_empty());
}

#[test]
fn effect_flagged_in_a_component_file() {
    let src = "export function Widget() {\n  useEffect(() => {}, []);\n  return <div/>;\n}\n";
    assert_eq!(tsx(src), vec!["effect-in-component"]);
}

#[test]
fn effect_ok_in_a_hook_file_without_a_component() {
    // A pure custom hook (no component) may use useEffect freely.
    let src = "export function useThing() {\n  useEffect(() => {}, []);\n  return 42;\n}\n";
    assert!(tsx(src).is_empty());
}

#[test]
fn effect_honours_line_allow() {
    let src = "export function Widget() {\n  useEffect(() => {}, []); // straitjacket-allow:effect-in-component\n  return <div/>;\n}\n";
    assert!(tsx(src).is_empty());
}

#[test]
fn effect_ok_in_a_hook_beside_a_component() {
    // The refined rule: an effect inside a `use*` hook is fine even in the same file as
    // a component. Only an effect *defined in the component body* is flagged.
    let src = "export function useThing() {\n  useEffect(() => {}, []);\n  return 1;\n}\nexport function Widget() {\n  return <div>{useThing()}</div>;\n}\n";
    assert!(tsx(src).is_empty());
}

#[test]
fn effect_flags_only_the_one_in_the_component() {
    // A component with an inline effect (flagged) beside a hook with an effect (fine):
    // exactly one finding, for the component's effect.
    let src = "export function useThing() {\n  useEffect(() => {}, []);\n  return 1;\n}\nexport function Widget() {\n  useEffect(() => {}, []);\n  return <div/>;\n}\n";
    assert_eq!(tsx(src), vec!["effect-in-component"]);
}

#[test]
fn effect_flagged_in_a_memo_component() {
    // An anonymous component (memo/forwardRef) still counts — an inline effect flags.
    let src =
        "export const Widget = memo(() => {\n  useEffect(() => {}, []);\n  return <div/>;\n});\n";
    assert_eq!(tsx(src), vec!["effect-in-component"]);
}

const CHILD_VALUE: &str =
    "export function Child({ value }: { value: number }) {\n  return <div>{value}</div>;\n}\n";

#[test]
fn prop_drilling_flags_a_pure_conduit() {
    // Panel receives `value`, never uses it, only forwards it to a local child.
    let src = "export function Panel({ value }) {\n  return <Child value={value} />;\n}\n";
    assert!(tsx_with(src, &[CHILD_VALUE]).contains(&"prop-drilling".to_string()));
}

#[test]
fn prop_drilling_ok_when_prop_is_also_used() {
    // Panel reads `value` AND forwards it — used at this stage, so not a conduit.
    let src = "export function Panel({ value }) {\n  return <div title={String(value)}><Child value={value} /></div>;\n}\n";
    assert!(!tsx_with(src, &[CHILD_VALUE]).contains(&"prop-drilling".to_string()));
}

#[test]
fn prop_drilling_ignores_library_component() {
    // `Button` isn't defined in our tree → it's a library component that must receive
    // props → not drilling.
    let src = "export function Panel({ value }) {\n  return <Button value={value} />;\n}\n";
    assert!(!tsx(src).contains(&"prop-drilling".to_string()));
}

#[test]
fn prop_drilling_ignores_callback_by_type() {
    // The target slot is typed as a function → a callback → fine, even forwarded.
    let src = "export function Panel({ onClose }) {\n  return <Child onClose={onClose} />;\n}\n";
    let child =
        "export function Child({ onClose }: { onClose: () => void }) {\n  return <button />;\n}\n";
    assert!(!tsx_with(src, &[child]).contains(&"prop-drilling".to_string()));
}

#[test]
fn prop_drilling_ok_when_prop_is_modified() {
    // A member/computed expression is a modification, not a passthrough.
    let a = "export function Panel({ user }) {\n  return <Child name={user.name} />;\n}\n";
    assert!(!tsx_with(a, &[CHILD_VALUE]).contains(&"prop-drilling".to_string()));
    let b = "export function Panel({ n }) {\n  return <Child n={n + 1} />;\n}\n";
    assert!(!tsx_with(b, &[CHILD_VALUE]).contains(&"prop-drilling".to_string()));
}

#[test]
fn prop_drilling_ok_on_a_dom_element() {
    // Binding a received prop to a DOM element isn't drilling.
    let src = "export function Panel({ value }) {\n  return <input value={value} />;\n}\n";
    assert!(!tsx_with(src, &[CHILD_VALUE]).contains(&"prop-drilling".to_string()));
}

#[test]
fn usestate_passed_one_hop_is_allowed() {
    // The relaxed model: useState → a direct child is fine (not a param forward).
    let src = "import {useState} from 'react';\nexport function App() {\n  const [count, setCount] = useState(0);\n  return <Child value={count} />;\n}\n";
    let hits = tsx_with(src, &[CHILD_VALUE]);
    assert!(!hits.contains(&"prop-drilling".to_string()));
    assert!(!hits.contains(&"store-passthrough".to_string()));
}

#[test]
fn store_passthrough_flags_unchanged_forward() {
    let src = "export function App() {\n  const user = useUserStore(s => s.user);\n  return <Profile user={user} />;\n}\n";
    let child = "export function Profile({ user }: { user: object }) {\n  return <div />;\n}\n";
    assert!(tsx_with(src, &[child]).contains(&"store-passthrough".to_string()));
}

#[test]
fn store_passthrough_ok_when_modified() {
    let src = "export function App() {\n  const user = useUserStore(s => s.user);\n  return <Profile name={user.name} />;\n}\n";
    let child = "export function Profile({ name }: { name: string }) {\n  return <div />;\n}\n";
    assert!(!tsx_with(src, &[child]).contains(&"store-passthrough".to_string()));
}

// ---- prop-drilling depth graph -----------------------------------------------

#[test]
fn extract_edges_finds_a_forward() {
    let src = "export function Row({ task }) {\n  return <TaskLinks task={task} />;\n}\n";
    let edges = straitjacket::react::extract_edges(src, "Row.tsx");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].from_component, "Row");
    assert_eq!(edges[0].from_param, "task");
    assert_eq!(edges[0].to_component, "TaskLinks");
    assert_eq!(edges[0].to_param, "task");
}

#[test]
fn chains_measure_drill_depth() {
    use straitjacket::prop_graph::{chains, Edge};
    let e = |fc: &str, fp: &str, tc: &str, tp: &str| Edge {
        from_component: fc.into(),
        from_param: fp.into(),
        to_component: tc.into(),
        to_param: tp.into(),
        file: "f".into(),
        line: 1,
    };
    // task drilled A → B → C → D (three forwarding hops).
    let edges = vec![
        e("A", "task", "B", "task"),
        e("B", "task", "C", "task"),
        e("C", "task", "D", "task"),
    ];
    let cs = chains(&edges);
    assert_eq!(cs.len(), 1);
    assert_eq!(cs[0].len(), 3); // depth 3
}

#[test]
fn react_rules_can_be_skipped() {
    let mut e = engine();
    e.skip(&[
        "one-component".to_string(),
        "effect-in-component".to_string(),
    ]);
    let src = "export function Widget() {\n  useEffect(() => {}, []);\n  return <div/>;\n}\nexport const Sidebar = () => <i/>;\n";
    assert!(e
        .scan_text(src, "C.tsx", "tsx")
        .iter()
        .all(|f| f.rule != "one-component" && f.rule != "effect-in-component"));
}

#[test]
fn duplication_is_on_by_default_and_listed() {
    assert!(engine().duplication().is_some());
    assert!(engine().rule_ids().contains(&"duplication"));
}

#[test]
fn duplication_skip_reports_no_false_unknown() {
    let mut e = engine();
    let unknown = e.skip(&["duplication".to_string()]);
    assert!(
        unknown.is_empty(),
        "skipping a real rule must not warn: {unknown:?}"
    );
    assert!(e.duplication().is_none());
}
