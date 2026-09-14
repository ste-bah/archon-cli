//! Issue-13: a tree-wide formatter run by a write agent must not fail its wave.
//!
//! Live on wf-7db01ce7 `agents-3-0`, the coder ran `cargo fmt --all` and left
//! sixty-four files outside its `target_files` changed; an undeclared path is
//! refused by gate 1 when reported and by gate 2 at capture when not, so the
//! branch's real work never reached the canonical tree and every dependent
//! wave was skipped. These drive the production write-wave seam with real Git
//! writes and prove that a whitespace-only change outside the scope is dropped
//! — restored in the worktree, excluded from the patch, reported for review —
//! while the branch's real work still commits.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::json;
use support::{Edits, FORMATTED_BASELINE, Fixture, git};

const REFORMATTED: &str = "fn f() {\n\t1\n}\n\n";

fn declared_targets(manifest: &serde_json::Value) -> Vec<String> {
    manifest["declared_target_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

fn whitespace_gap(result: &WorkflowV2Result, branch_id: &str) -> WorkflowV2ResidualGap {
    result
        .residual_gaps
        .iter()
        .find(|gap| gap.id == format!("whitespace_only_changes_dropped_{branch_id}"))
        .cloned()
        .unwrap_or_else(|| panic!("no whitespace-drop gap: {result:#?}"))
}

/// (a) The live shape in miniature, reported: one declared file changed, one
/// undeclared file re-indented and named in the envelope. The branch is
/// accepted, its real work commits, the re-indented file stays at baseline,
/// the manifest declares only the real file, and the drop is a review gap.
#[tokio::test]
async fn a_reported_whitespace_only_change_outside_scope_is_dropped_not_fatal() {
    let f = Fixture::new();
    let out = f
        .wave(
            "format",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        ("src/formatted.txt", REFORMATTED),
                    ],
                    report: vec!["owned.txt", "src/formatted.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_ne!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    assert_eq!(
        std::fs::read_to_string(f.repo.join("src/formatted.txt")).unwrap(),
        FORMATTED_BASELINE,
        "formatter noise must never reach the canonical tree"
    );
    assert_eq!(
        git(
            &f.repo,
            &["status", "--porcelain", "--", "src", "owned.txt"]
        ),
        ""
    );
    let manifest = f.manifest("format", "format-0");
    assert_eq!(declared_targets(&manifest), vec!["owned.txt".to_string()]);
    assert_eq!(manifest["changed_files"], json!(["owned.txt"]));
    let result = f.branch_result("format", "format-0");
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    let gap = whitespace_gap(&result, "format-0");
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(gap.description.contains("src/formatted.txt"), "{gap:?}");
    assert!(gap.description.contains("whitespace only"), "{gap:?}");
    assert_eq!(
        result.data["whitespace_only_dropped"],
        json!(["src/formatted.txt"])
    );
    assert!(
        result.evidence.iter().any(|e| e
            .summary
            .contains("whitespace-only changes outside the declared targets dropped")
            && e.summary.contains("src/formatted.txt")),
        "{result:#?}"
    );
    assert!(result.data.get("scope_granted").is_none(), "{result:#?}");
    assert_eq!(result.data["patch_landed"], json!(true), "{result:#?}");
    assert!(
        !result
            .files_changed
            .iter()
            .any(|file| file.path.contains("formatted")),
        "the dropped path must leave the envelope too: {result:#?}"
    );
}

/// (e) The live shape UNREPORTED: the formatter touched the file, the agent
/// did not list it. Gate 2 would have raised `UndeclaredWrite` at capture;
/// the drop restores the file before capture reads the worktree, and the
/// branch still commits its real work.
#[tokio::test]
async fn an_unreported_whitespace_only_change_outside_scope_still_lets_the_work_commit() {
    let f = Fixture::new();
    let out = f
        .wave(
            "quiet",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        ("src/formatted.txt", REFORMATTED),
                    ],
                    report: vec!["owned.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:src/formatted.txt"]),
        FORMATTED_BASELINE.trim_end()
    );
    let result = f.branch_result("quiet", "quiet-0");
    let gap = whitespace_gap(&result, "quiet-0");
    assert!(gap.description.contains("src/formatted.txt"), "{gap:?}");
    let manifest = f.manifest("quiet", "quiet-0");
    assert_eq!(manifest["changed_files"], json!(["owned.txt"]));
}

/// (b) A DECLARED file with a whitespace-only change is the item's own work:
/// still captured, still committed, never reported as a drop.
#[tokio::test]
async fn a_declared_file_reformatted_by_whitespace_only_is_still_included() {
    let f = Fixture::new();
    let out = f
        .wave(
            "declared",
            vec![(
                vec!["owned.txt", "src/formatted.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        ("src/formatted.txt", REFORMATTED),
                    ],
                    report: vec!["owned.txt", "src/formatted.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        std::fs::read_to_string(f.repo.join("src/formatted.txt")).unwrap(),
        REFORMATTED
    );
    let manifest = f.manifest("declared", "declared-0");
    assert_eq!(
        manifest["changed_files"],
        json!(["owned.txt", "src/formatted.txt"])
    );
    let result = f.branch_result("declared", "declared-0");
    assert!(
        !result
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with("whitespace_only_changes_dropped_")),
        "{result:#?}"
    );
    assert!(result.data.get("whitespace_only_dropped").is_none());
}

/// (c) An undeclared file with a REAL change beside a whitespace-only one:
/// the real one is granted as unclaimed scope exactly as today, the
/// whitespace-only one is dropped, and the two never get confused.
#[tokio::test]
async fn a_real_undeclared_change_is_still_granted_while_the_whitespace_one_is_dropped() {
    let f = Fixture::new();
    let out = f
        .wave(
            "mixed",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        ("forgotten.txt", "also needed\n"),
                        ("src/formatted.txt", REFORMATTED),
                    ],
                    report: vec!["owned.txt", "forgotten.txt", "src/formatted.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:forgotten.txt"]), "also needed");
    assert_eq!(
        std::fs::read_to_string(f.repo.join("src/formatted.txt")).unwrap(),
        FORMATTED_BASELINE
    );
    let manifest = f.manifest("mixed", "mixed-0");
    let mut declared = declared_targets(&manifest);
    declared.sort();
    assert_eq!(
        declared,
        vec!["forgotten.txt".to_string(), "owned.txt".to_string()]
    );
    let result = f.branch_result("mixed", "mixed-0");
    assert_eq!(result.data["scope_granted"], json!(["forgotten.txt"]));
    assert_eq!(
        result.data["whitespace_only_dropped"],
        json!(["src/formatted.txt"])
    );
}

/// A whitespace-only change to a file the OTHER item declares is dropped
/// too, not contested: there is no ownership dispute over formatter noise,
/// and both branches commit.
#[tokio::test]
async fn a_whitespace_only_change_to_another_items_file_is_dropped_not_contested() {
    let f = Fixture::new();
    let out = f
        .wave(
            "pair",
            vec![
                (
                    vec!["owned.txt"],
                    Edits {
                        files: vec![
                            ("owned.txt", "implemented\n"),
                            ("other.txt", "other  baseline\n"),
                        ],
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
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    assert_eq!(git(&f.repo, &["show", "HEAD:other.txt"]), "theirs");
    let result = f.branch_result("pair", "pair-0");
    assert_eq!(result.data["whitespace_only_dropped"], json!(["other.txt"]));
}
