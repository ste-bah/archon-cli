//! Issue-11: the unclaimed-path grant must reach ownership gate 1.
//!
//! Live on wf-5979fe15 `agents-5`, a single-item wave reported two files it
//! had not declared and nobody else claimed; gate 1 refused the envelope
//! against the DECLARED targets before capture — the only place the grant
//! was applied — ever ran, and eleven dependent waves were skipped. These
//! drive the production write-wave seam with real Git writes and prove the
//! grant is judged once, by every gate, with the same answer.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

/// (a) The live shape in miniature: one item, one declared file changed, one
/// undeclared file nobody else claims. Gate 1 accepts, capture includes the
/// file, the manifest declares it, and the result says it was granted.
#[tokio::test]
async fn single_item_wave_grants_an_unclaimed_file_through_every_gate() {
    let f = Fixture::new();
    let out = f
        .wave(
            "grant",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        ("forgotten.txt", "also needed\n"),
                    ],
                    report: vec!["owned.txt", "forgotten.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_ne!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    assert_eq!(git(&f.repo, &["show", "HEAD:forgotten.txt"]), "also needed");
    let manifest = f.manifest("grant", "grant-0");
    let declared: Vec<&str> = manifest["declared_target_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(declared.contains(&"forgotten.txt"), "{declared:?}");
    let result = f.branch_result("grant", "grant-0");
    assert_eq!(
        result.data["scope_granted"],
        json!(["forgotten.txt"]),
        "{result:#?}"
    );
    assert!(
        result.evidence.iter().any(
            |e| e.summary.contains("scope granted beyond declared targets")
                && e.summary.contains("forgotten.txt")
        ),
        "{result:#?}"
    );
}

/// (b) The same undeclared file, but the OTHER item in the wave declared it.
/// A contested path is refused exactly as before, by the same message.
#[tokio::test]
async fn two_item_wave_still_rejects_a_file_the_other_item_claims() {
    let f = Fixture::new();
    let out = f
        .wave(
            "contest",
            vec![
                (
                    vec!["owned.txt"],
                    Edits {
                        files: vec![("owned.txt", "implemented\n"), ("other.txt", "trespass\n")],
                        report: vec!["owned.txt", "other.txt"],
                        via_adapter: false,
                    },
                ),
                (
                    vec!["other.txt"],
                    Edits {
                        files: vec![("other.txt", "theirs\n")],
                        report: vec!["other.txt"],
                        via_adapter: false,
                    },
                ),
            ],
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    let gap = f.branch_gap("contest", "contest-0");
    assert_eq!(
        gap,
        "write item 'contest-0' changed undeclared path 'other.txt'"
    );
    assert_ne!(git(&f.repo, &["show", "HEAD:other.txt"]), "trespass");
    let result = f.branch_result("contest", "contest-0");
    assert!(result.data.get("scope_granted").is_none(), "{result:#?}");
}

// (c) A whitespace-only change outside the scope is no longer refused here:
// Issue-13 drops it. See `write_wave_whitespace_drop.rs`.

/// (d) The live envelope shape: nine files reported, two undeclared, single
/// item, nothing else claims them. Every gate passes and all nine land.
#[tokio::test]
async fn the_live_nine_file_envelope_with_two_undeclared_paths_lands() {
    let f = Fixture::new();
    let declared = vec![
        "crates/t/src/lib.rs",
        "crates/t/src/validation.rs",
        "crates/t/src/validation/checks.rs",
        "crates/t/src/validation/report.rs",
        "crates/t/tests/native_interval_gates.rs",
        "crates/t/tests/validation_report.rs",
        "crates/t/tests/validation_rules.rs",
    ];
    let undeclared = ["crates/t/src/data_store.rs", "crates/t/src/ohlcv.rs"];
    let files: Vec<(&str, &str)> = declared
        .iter()
        .chain(undeclared.iter())
        .map(|path| (*path, "// implemented\n"))
        .collect();
    let report: Vec<&str> = files.iter().map(|(path, _)| *path).collect();
    assert_eq!(report.len(), 9);
    let out = f
        .wave(
            "live",
            vec![(
                declared.clone(),
                Edits {
                    files,
                    report: report.clone(),
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    for path in &report {
        assert_eq!(
            git(&f.repo, &["show", &format!("HEAD:{path}")]),
            "// implemented",
            "{path}"
        );
    }
    let result = f.branch_result("live", "live-0");
    assert_eq!(
        result.data["scope_granted"],
        json!(undeclared),
        "{result:#?}"
    );
    let manifest = f.manifest("live", "live-0");
    let manifest_declared = manifest["declared_target_files"].to_string();
    for path in undeclared {
        assert!(manifest_declared.contains(path), "{manifest_declared}");
    }
}
