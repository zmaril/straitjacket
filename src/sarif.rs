//! SARIF 2.1.0 output, for GitHub code scanning: uploaded via
//! `github/codeql-action/upload-sarif`, each finding renders as an annotation on the
//! line it points at (and in the repo's Security tab).
//!
//! straitjacket-allow-file:deep-nesting — the document is one hand-written
//! `json!({ … })` literal, so the indentation is structural nesting of data, not
//! deeply nested logic.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::finding::{Finding, Severity};

fn level(severity: &Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

/// Render findings as a SARIF 2.1.0 document. `version` is straitjacket's own version
/// (for the tool driver).
pub fn to_sarif(findings: &[Finding], version: &str) -> String {
    // One rule descriptor per distinct rule id (stable order), using a finding's
    // message as the short description.
    let mut rule_msg: BTreeMap<&str, &str> = BTreeMap::new();
    for f in findings {
        rule_msg
            .entry(f.rule.as_str())
            .or_insert(f.message.as_str());
    }
    let rules: Vec<Value> = rule_msg
        .iter()
        .map(|(id, msg)| {
            json!({
                "id": id,
                "shortDescription": { "text": msg },
                "helpUri": "https://straitjacket.dev/docs/reference/rules",
            })
        })
        .collect();

    let results: Vec<Value> = findings
        .iter()
        .map(|f| {
            json!({
                "ruleId": f.rule,
                "level": level(&f.severity),
                "message": { "text": f.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": f.path },
                        "region": {
                            "startLine": f.line,
                            "startColumn": f.col,
                            "snippet": { "text": f.matched },
                        },
                    },
                }],
            })
        })
        .collect();

    let doc = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "straitjacket",
                "informationUri": "https://straitjacket.dev",
                "version": version,
                "rules": rules,
            }},
            "results": results,
        }],
    });
    serde_json::to_string_pretty(&doc).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(rule: &str, sev: Severity) -> Finding {
        Finding {
            rule: rule.into(),
            path: "src/a.ts".into(),
            line: 12,
            col: 4,
            matched: "x".into(),
            message: "why".into(),
            severity: sev,
        }
    }

    #[test]
    fn well_formed_sarif() {
        let out = to_sarif(&[finding("emoji", Severity::Error)], "9.9.9");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["version"], "2.1.0");
        assert_eq!(v["runs"][0]["tool"]["driver"]["name"], "straitjacket");
        assert_eq!(v["runs"][0]["tool"]["driver"]["version"], "9.9.9");
        let result = &v["runs"][0]["results"][0];
        assert_eq!(result["ruleId"], "emoji");
        assert_eq!(result["level"], "error");
        let region = &result["locations"][0]["physicalLocation"]["region"];
        assert_eq!(region["startLine"], 12);
        assert_eq!(region["startColumn"], 4);
        assert_eq!(
            v["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]
                ["uri"],
            "src/a.ts"
        );
    }

    #[test]
    fn warning_maps_to_warning_level() {
        let out = to_sarif(&[finding("slop-prose", Severity::Warning)], "0");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["runs"][0]["results"][0]["level"], "warning");
    }

    #[test]
    fn dedups_rule_descriptors() {
        let out = to_sarif(
            &[
                finding("emoji", Severity::Error),
                finding("emoji", Severity::Error),
            ],
            "0",
        );
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["runs"][0]["tool"]["driver"]["rules"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(v["runs"][0]["results"].as_array().unwrap().len(), 2);
    }
}
