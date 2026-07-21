//! Tests for the `unused-marker` check — straitjacket's analogue of clippy's unused
//! `#[allow]`. A `straitjacket-allow[-file]` marker that suppresses nothing is an error.
//!
//! The engine-level cases drive the reconciliation seam directly (`collect_markers` +
//! `scan_text_candidates` diffed against `scan_text`, fed to `unused_marker_findings`); the
//! CLI cases exercise the whole pipeline, including the default-on config flag and the
//! cross-file `duplication` wrong-side detection.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use straitjacket::engine::{collect_markers, suppressed_between, Marker, Suppressed};
use straitjacket::{Config, Engine, Finding};

/// The `unused-marker` findings for a single file's per-file rules, computed the way `main`
/// does: diff the candidate scan against the visible scan to learn what each marker
/// suppressed, then reconcile. `dup_partner` is `None` (no cross-file duplication here).
fn unused(e: &Engine, src: &str, ext: &str) -> Vec<Finding> {
    let markers = collect_markers(src);
    let visible = e.scan_text(src, "t", ext);
    let candidates = e.scan_text_candidates(src, "t", ext);
    let suppressed = suppressed_between(&candidates, &visible);
    e.unused_marker_findings("t", ext, &markers, &suppressed, None)
}

fn engine() -> Engine {
    Engine::new(&Config::default()).expect("rules compile")
}

// ---- engine-level reconciliation --------------------------------------------

#[test]
fn file_marker_naming_a_rule_with_no_finding_is_flagged() {
    // `color` applies to `.ts`, but the file has no color anywhere, so the marker is dead.
    let src = "// straitjacket-allow-file:color\nconst x = 1;\n";
    let found = unused(&engine(), src, "ts");
    assert_eq!(found.len(), 1, "expected one unused-marker, got {found:?}");
    assert_eq!(found[0].rule, "unused-marker");
    assert_eq!(found[0].line, 1);
    assert!(
        found[0].message.contains("color"),
        "message should name the rule: {}",
        found[0].message
    );
}

#[test]
fn bracket_marker_on_a_clean_file_is_flagged_as_all_rules() {
    // `straitjacket-allow-file[:duplication]` resolves to *all rules* (the `[:` is not a
    // scope), so on a file with no findings it suppressed nothing.
    let src = "/* straitjacket-allow-file[:duplication] */\nconst x = 1;\n";
    let found = unused(&engine(), src, "css");
    assert_eq!(found.len(), 1, "expected one unused-marker, got {found:?}");
    assert_eq!(found[0].line, 1);
    assert!(
        found[0].message.contains("all rules"),
        "a bare/all marker should name 'all rules': {}",
        found[0].message
    );
}

#[test]
fn a_marker_that_suppresses_a_finding_is_not_flagged() {
    // The file-level color marker genuinely silences the `#fff` on line 2, so it is used.
    let src = "// straitjacket-allow-file:color\nconst c = '#fff';\n";
    assert!(
        unused(&engine(), src, "ts").is_empty(),
        "a used marker must not be flagged"
    );
}

#[test]
fn a_marker_for_an_inapplicable_rule_is_inert_not_flagged() {
    // `color` never runs on Markdown, so a `color` marker there can't suppress anything —
    // it's inert, and must not be reported as unused (this is what keeps docs quiet).
    let src = "<!-- straitjacket-allow-file:color -->\njust prose\n";
    assert!(unused(&engine(), src, "md").is_empty());
}

#[test]
fn wrong_side_duplication_marker_message_points_at_the_first_file() {
    // A `duplication` marker on the *second* file of a clone pair is dead by construction —
    // suppression only reads the first file. With a partner set, the message says so.
    let markers = vec![Marker {
        line: 1,
        file_level: true,
        rule: Some("duplication".to_string()),
    }];
    let found = engine().unused_marker_findings("b.rs", "rs", &markers, &[], Some("a.rs"));
    assert_eq!(found.len(), 1);
    assert!(
        found[0].message.contains("second file") && found[0].message.contains("a.rs"),
        "expected a wrong-side message naming a.rs, got {}",
        found[0].message
    );
}

#[test]
fn a_used_duplication_marker_is_not_flagged() {
    // The same marker, but it actually suppressed a clone on this file — so it is used.
    let markers = vec![Marker {
        line: 1,
        file_level: true,
        rule: Some("duplication".to_string()),
    }];
    let suppressed = vec![Suppressed {
        rule: "duplication".to_string(),
        line: 1,
    }];
    let found = engine().unused_marker_findings("a.rs", "rs", &markers, &suppressed, None);
    assert!(
        found.is_empty(),
        "a used duplication marker must not be flagged"
    );
}

// ---- CLI end-to-end ---------------------------------------------------------

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_straitjacket"))
}

/// Write `files` into a fresh temp dir and return it. Caller removes it.
fn scratch(tag: &str, files: &[(&str, String)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sj-um-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for (name, contents) in files {
        fs::write(dir.join(name), contents).unwrap();
    }
    dir
}

/// A block big enough to trip the default duplication threshold on its own.
fn big_block(name: &str) -> String {
    let mut s = format!("fn {name}() {{\n");
    for i in 0..12 {
        s.push_str(&format!("    let v{i} = {i} * 3 + 1;\n"));
    }
    s.push_str("    println!(\"{}\", v0 + v1 + v2 + v3 + v4 + v5);\n}\n");
    s
}

#[test]
fn cli_flags_a_dead_marker_and_fails() {
    let dir = scratch(
        "dead",
        &[(
            "only.ts",
            "// straitjacket-allow-file:color\nconst x = 1;\n".to_string(),
        )],
    );
    let out = bin()
        .arg(&dir)
        .arg("--no-config")
        .output()
        .expect("run straitjacket");
    let _ = fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("unused-marker"),
        "expected an unused-marker finding, got:\n{stdout}"
    );
    assert!(!out.status.success(), "a dead marker must fail the run");
}

#[test]
fn cli_flag_off_silences_the_check() {
    let dir = scratch(
        "off",
        &[(
            "only.ts",
            "// straitjacket-allow-file:color\nconst x = 1;\n".to_string(),
        )],
    );
    let out = bin()
        .arg(&dir)
        .arg("--no-config")
        .arg("--no-fail-on-unused-markers")
        .output()
        .expect("run straitjacket");
    let _ = fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("unused-marker"),
        "the check should be silent when off, got:\n{stdout}"
    );
    assert!(
        out.status.success(),
        "with no other findings and the check off, the run should pass"
    );
}

#[test]
fn cli_flags_a_wrong_side_duplication_marker() {
    // Identical block in two files; the marker is on the *second* (alphabetically-later)
    // file, where the detector never reads it — so the clone still fires AND the marker is
    // reported as a wrong-side unused marker.
    let block = big_block("shared");
    let dir = scratch(
        "wrongside",
        &[
            ("a_first.rs", block.clone()),
            (
                "b_second.rs",
                format!("// straitjacket-allow-file:duplication\n{block}"),
            ),
        ],
    );
    let out = bin()
        .arg(&dir)
        .arg("--no-config")
        .output()
        .expect("run straitjacket");
    let _ = fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("b_second.rs") && stdout.contains("unused-marker"),
        "expected a wrong-side unused-marker on b_second.rs, got:\n{stdout}"
    );
}

#[test]
fn cli_does_not_flag_a_marker_on_the_first_file() {
    // The same clone, but the marker sits on the first file, where it genuinely suppresses
    // the clone — so it is used, and there is no unused-marker report.
    let block = big_block("shared");
    let dir = scratch(
        "rightside",
        &[
            (
                "a_first.rs",
                format!("// straitjacket-allow-file:duplication\n{block}"),
            ),
            ("b_second.rs", block.clone()),
        ],
    );
    let out = bin()
        .arg(&dir)
        .arg("--no-config")
        .output()
        .expect("run straitjacket");
    let _ = fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("unused-marker"),
        "a marker on the first file is used, not unused, got:\n{stdout}"
    );
}

#[test]
fn cli_both_sides_marked_credits_only_the_first_file() {
    // A clone pair where BOTH files carry `straitjacket-allow-file:duplication`. Suppression is
    // asymmetric: the detector reads the marker only on `fragment_a` (the alphabetically-first
    // file), so that marker is load-bearing (it suppresses the clone) while the one on
    // `fragment_b` is inert by construction. The check must credit `a_first.rs` (no unused
    // marker) and flag `b_second.rs` with the wrong-side message.
    //
    // Crucially this runs with a RELATIVE scan path (`cwd = dir`, arg `.`): the cross-file
    // duplication pass reports canonical absolute paths, while the per-file scan keys markers by
    // relative display path, and the two namespaces must still be reconciled. An absolute scan
    // path (as the other CLI cases here use) accidentally makes the two forms coincide and hides
    // the join bug this guards against.
    let block = big_block("shared");
    let dir = scratch(
        "bothsides",
        &[
            (
                "a_first.rs",
                format!("// straitjacket-allow-file:duplication\n{block}"),
            ),
            (
                "b_second.rs",
                format!("// straitjacket-allow-file:duplication\n{block}"),
            ),
        ],
    );
    let out = bin()
        .current_dir(&dir)
        .arg(".")
        .arg("--no-config")
        .arg("--format")
        .arg("json")
        .output()
        .expect("run straitjacket");
    let _ = fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let findings: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("parse json {e}:\n{stdout}"));
    let findings = findings.as_array().expect("findings array");

    let unused_on = |file: &str| -> Vec<&serde_json::Value> {
        findings
            .iter()
            .filter(|f| {
                f["rule"] == "unused-marker"
                    && f["path"].as_str().is_some_and(|p| p.ends_with(file))
            })
            .collect()
    };

    // `fragment_a`'s marker is load-bearing — it must NOT be flagged.
    assert!(
        unused_on("a_first.rs").is_empty(),
        "the first file's marker suppresses the clone and must be used, got:\n{stdout}"
    );
    // `fragment_b`'s marker is inert — flagged, and with the wrong-side message naming the
    // first file (not the generic "suppressed no findings" one).
    let b = unused_on("b_second.rs");
    assert_eq!(
        b.len(),
        1,
        "the second file's marker is inert and must be flagged once, got:\n{stdout}"
    );
    let msg = b[0]["message"].as_str().unwrap_or_default();
    assert!(
        msg.contains("second file") && msg.contains("a_first.rs"),
        "expected the wrong-side message naming a_first.rs, got: {msg}"
    );
}
