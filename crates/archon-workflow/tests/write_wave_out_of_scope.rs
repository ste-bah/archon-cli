//! Issue-27: a write branch's grant stops at its plan's scope roots.
//!
//! Live on wf-719ff3b0 `agents-11`, a single-item wave declared targets in
//! `crates/archon-trading/` and the coder edited twenty files under two
//! unrelated crates; nothing contested them, so all twenty were granted,
//! declared and committed under the task. This drives the production
//! write-wave seam with real Git writes and proves that a real change in an
//! undeclared crate is dropped — restored or removed in the worktree,
//! excluded from the patch and the manifest, reported for review naming the
//! roots — while the branch's in-scope work, its in-scope unclaimed siblings
//! and a root-level shared file all still land.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

fn declared_targets(manifest: &serde_json::Value) -> Vec<String> {
    manifest["declared_target_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

fn out_of_scope_gap(result: &WorkflowV2Result, branch_id: &str) -> WorkflowV2ResidualGap {
    result
        .residual_gaps
        .iter()
        .find(|gap| gap.id == format!("out_of_scope_changes_dropped_{branch_id}"))
        .cloned()
        .unwrap_or_else(|| panic!("no out-of-scope gap: {result:#?}"))
}

/// Two crates marked by `Cargo.toml`, and a workspace lockfile at the root.
fn with_two_crates(f: &Fixture) {
    for (path, content) in [
        ("Cargo.lock", "# lock\n"),
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        ("crates/a/src/lib.rs", "// a\n"),
        ("crates/b/Cargo.toml", "[package]\nname = \"b\"\n"),
        ("crates/b/src/lib.rs", "// b\n"),
    ] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "two crates"]);
}

/// The live shape in miniature: one item declaring a file in crate `a`
/// edits it, creates a sibling in `a`, touches the root lockfile, and edits
/// and creates files in crate `b` — reporting all of it. Crate `a` and the
/// lockfile land; crate `b` is untouched in the canonical tree, absent from
/// the manifest and the envelope, and named in the review gap with the roots.
#[tokio::test]
async fn a_change_in_an_undeclared_crate_is_dropped_and_the_in_scope_work_lands() {
    let f = Fixture::new();
    with_two_crates(&f);
    let (out, prompts) = f
        .wave_audited(
            "crate",
            vec![(
                vec!["crates/a/src/lib.rs"],
                Edits {
                    files: vec![
                        ("crates/a/src/lib.rs", "// a implemented\n"),
                        ("crates/a/src/extra.rs", "// a sibling\n"),
                        ("Cargo.lock", "# lock updated\n"),
                        ("crates/b/src/lib.rs", "// b clippy fix\n"),
                        ("crates/b/src/new.rs", "// b created\n"),
                    ],
                    report: vec![
                        "crates/a/src/lib.rs",
                        "crates/a/src/extra.rs",
                        "Cargo.lock",
                        "crates/b/src/lib.rs",
                        "crates/b/src/new.rs",
                    ],
                    via_adapter: true,
                },
            )],
            None,
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/a/src/lib.rs"]),
        "// a implemented"
    );
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/a/src/extra.rs"]),
        "// a sibling"
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:Cargo.lock"]), "# lock updated");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/b/src/lib.rs"]),
        "// b",
        "an undeclared crate must never reach the canonical tree"
    );
    assert!(!f.repo.join("crates/b/src/new.rs").exists());
    assert_eq!(
        git(
            &f.repo,
            &["status", "--porcelain", "--", "crates", "Cargo.lock"]
        ),
        ""
    );
    let manifest = f.manifest("crate", "crate-0");
    let mut declared = declared_targets(&manifest);
    declared.sort();
    assert_eq!(
        declared,
        vec![
            "Cargo.lock".to_string(),
            "crates/a/src/extra.rs".to_string(),
            "crates/a/src/lib.rs".to_string(),
        ]
    );
    assert_eq!(
        manifest["changed_files"],
        json!(["Cargo.lock", "crates/a/src/extra.rs", "crates/a/src/lib.rs"])
    );
    let result = f.branch_result("crate", "crate-0");
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    let gap = out_of_scope_gap(&result, "crate-0");
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(gap.description.contains("2 path(s)"), "{gap:?}");
    assert!(gap.description.contains("crates/b/src/lib.rs"), "{gap:?}");
    assert!(gap.description.contains("crates/b/src/new.rs"), "{gap:?}");
    assert!(
        gap.description.contains("scope roots (crates/a/)"),
        "{gap:?}"
    );
    assert!(
        gap.description.ends_with(
            "a needed change elsewhere is a residual gap to report, not an edit to make."
        ),
        "{gap:?}"
    );
    assert_eq!(
        result.data["out_of_scope_dropped"],
        json!(["crates/b/src/lib.rs", "crates/b/src/new.rs"])
    );
    assert_eq!(
        result.data["scope_granted"],
        json!(["Cargo.lock", "crates/a/src/extra.rs"])
    );
    assert!(
        result
            .evidence
            .iter()
            .any(|e| e.summary.contains("out-of-scope changes dropped")
                && e.summary.contains("crates/b/src/new.rs")),
        "{result:#?}"
    );
    assert!(
        !result
            .files_changed
            .iter()
            .any(|file| file.path.contains("crates/b/")),
        "the dropped paths must leave the envelope too: {result:#?}"
    );
    assert_eq!(result.data["patch_landed"], json!(true), "{result:#?}");
    assert_eq!(prompts.len(), 1, "{prompts:#?}");
    assert!(
        prompts[0].contains("Scope roots: crates/a/. Changes outside these"),
        "the agent must be told the ceiling the gate applies: {}",
        prompts[0]
    );
}

/// An out-of-scope change the agent did NOT report is dropped just the
/// same, before capture would have raised `UndeclaredWrite` for it, and is
/// still counted as under-reporting.
#[tokio::test]
async fn an_unreported_change_in_an_undeclared_crate_is_dropped_before_capture() {
    let f = Fixture::new();
    with_two_crates(&f);
    let out = f
        .wave(
            "quiet",
            vec![(
                vec!["crates/a/src/lib.rs"],
                Edits {
                    files: vec![
                        ("crates/a/src/lib.rs", "// a implemented\n"),
                        ("crates/b/src/lib.rs", "// b clippy fix\n"),
                    ],
                    report: vec!["crates/a/src/lib.rs"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/a/src/lib.rs"]),
        "// a implemented"
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:crates/b/src/lib.rs"]), "// b");
    let result = f.branch_result("quiet", "quiet-0");
    assert_eq!(
        result.data["out_of_scope_dropped"],
        json!(["crates/b/src/lib.rs"])
    );
    assert_eq!(
        result.data["files_changed_underreported"],
        json!(["crates/b/src/lib.rs"])
    );
    assert!(result.data.get("scope_granted").is_none(), "{result:#?}");
    let manifest = f.manifest("quiet", "quiet-0");
    assert_eq!(manifest["changed_files"], json!(["crates/a/src/lib.rs"]));
}
