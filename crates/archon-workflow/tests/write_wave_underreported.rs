//! Issue-16: an agent that under-reports `files_changed` must not fail its wave.
//!
//! Live on wf-719ff3b0 `agents-5-0`, the worktree held twenty-two changed
//! files, the envelope listed fourteen, and nobody else in the wave claimed
//! the rest. The grant was computed from the envelope alone, so the unlisted
//! paths were never declared; gate 3 refused the first of them as an
//! undeclared write, the branch failed with `failure_kind: safety`, and the
//! stage with it. These drive the production write-wave seam with real Git
//! writes and prove that the grant is computed from the worktree — an
//! unlisted, unclaimed change is granted, lands, and is reported for review —
//! while a contested one is still refused.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

fn underreported_gap(result: &WorkflowV2Result, branch_id: &str) -> WorkflowV2ResidualGap {
    result
        .residual_gaps
        .iter()
        .find(|gap| gap.id == format!("files_changed_underreported_{branch_id}"))
        .cloned()
        .unwrap_or_else(|| panic!("no under-reporting gap: {result:#?}"))
}

/// (d) One item changes its declared file and an undeclared file nobody else
/// claims, and reports only the declared one. Both commit; the manifest
/// declares the granted path; the result carries it under `scope_granted`
/// and names it in a review gap.
#[tokio::test]
async fn an_unreported_unclaimed_change_lands_and_is_a_review_gap() {
    let f = Fixture::new();
    let out = f
        .wave(
            "quiet",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        ("forgotten.txt", "also needed\n"),
                    ],
                    report: vec!["owned.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_ne!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    assert_eq!(git(&f.repo, &["show", "HEAD:forgotten.txt"]), "also needed");
    let manifest = f.manifest("quiet", "quiet-0");
    assert!(
        manifest["declared_target_files"]
            .to_string()
            .contains("forgotten.txt"),
        "{manifest}"
    );
    assert_eq!(
        manifest["changed_files"],
        json!(["forgotten.txt", "owned.txt"]),
        "{manifest}"
    );
    let result = f.branch_result("quiet", "quiet-0");
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(result.data["scope_granted"], json!(["forgotten.txt"]));
    assert_eq!(result.data["patch_landed"], json!(true), "{result:#?}");
    let gap = underreported_gap(&result, "quiet-0");
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(gap.description.contains("forgotten.txt"), "{gap:?}");
    assert!(gap.description.contains("did not list"), "{gap:?}");
    assert_eq!(
        result.data["files_changed_underreported"],
        json!(["forgotten.txt"])
    );
    assert!(
        result
            .evidence
            .iter()
            .any(|e| e.summary.contains("files_changed under-reported")
                && e.summary.contains("forgotten.txt")),
        "{result:#?}"
    );
    assert!(
        result
            .residual_gaps
            .iter()
            .all(|gap| !gap.id.starts_with("invalid_write_branch_output_")),
        "under-reporting is a finding, not a failure: {result:#?}"
    );
}

/// (b) The unreported file is claimed by the other item: contested, refused
/// at capture exactly as before, and the trespass never reaches the tree.
/// The under-reporting gap still names it on the rejected result.
#[tokio::test]
async fn an_unreported_contested_change_is_still_refused() {
    let f = Fixture::new();
    let out = f
        .wave(
            "contest",
            vec![
                (
                    vec!["owned.txt"],
                    Edits {
                        files: vec![("owned.txt", "implemented\n"), ("other.txt", "trespass\n")],
                        report: vec!["owned.txt"],
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
    assert!(
        gap.contains("patch writes undeclared path 'other.txt'"),
        "{gap}"
    );
    assert_ne!(git(&f.repo, &["show", "HEAD:other.txt"]), "trespass");
    let result = f.branch_result("contest", "contest-0");
    assert!(result.data.get("scope_granted").is_none(), "{result:#?}");
    let gap = underreported_gap(&result, "contest-0");
    assert!(gap.description.contains("other.txt"), "{gap:?}");
}

/// (e) The live shape: twenty-two files changed, fourteen reported, eight
/// unreported and unclaimed, one item in the wave. Every gate passes and all
/// twenty-two land; the eight are granted and named.
#[tokio::test]
async fn the_live_fourteen_of_twenty_two_envelope_lands_in_full() {
    let f = Fixture::new();
    // `Edits` carries `&'static str`; leaking twenty-two test paths is fine.
    let changed: Vec<&'static str> = (0..22)
        .map(|i| &*Box::leak(format!("crates/t/src/f{i:02}.rs").into_boxed_str()))
        .collect();
    let files: Vec<(&str, &str)> = changed
        .iter()
        .map(|path| (*path, "// implemented\n"))
        .collect();
    let declared: Vec<&str> = changed[..10].to_vec();
    let report: Vec<&str> = changed[..14].to_vec();
    let out = f
        .wave(
            "live",
            vec![(
                declared,
                Edits {
                    files,
                    report,
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    for path in &changed {
        assert_eq!(
            git(&f.repo, &["show", &format!("HEAD:{path}")]),
            "// implemented",
            "{path}"
        );
    }
    let result = f.branch_result("live", "live-0");
    assert_eq!(
        result.data["scope_granted"],
        json!(changed[10..]),
        "{result:#?}"
    );
    assert_eq!(
        result.data["files_changed_underreported"],
        json!(changed[14..]),
        "{result:#?}"
    );
    let gap = underreported_gap(&result, "live-0");
    assert!(gap.description.contains("8 path(s)"), "{gap:?}");
    let manifest = f.manifest("live", "live-0");
    let manifest_declared = manifest["declared_target_files"].to_string();
    for path in &changed[10..] {
        assert!(manifest_declared.contains(path), "{manifest_declared}");
    }
}
