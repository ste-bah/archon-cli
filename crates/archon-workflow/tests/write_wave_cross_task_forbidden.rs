//! A cross-task write item — one branch remediating a finding two tasks
//! share — owns BOTH tasks' declared files, even though each task's forbidden
//! list names the other's file. The forbidden list of a multi-task item drops
//! a pattern only when everything it names lies inside a declared path of one
//! of its own tasks; a sibling's forbidden DIRECTORY stays in force for every
//! undeclared path under it, while the declared file in it stays writable.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

fn task(id: &str, owns: &str, forbids: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: vec![owns.into()],
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

fn two_crates(f: &Fixture) {
    for (path, content) in [
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        ("crates/a/src/lib.rs", "// a\n"),
        ("crates/b/Cargo.toml", "[package]\nname = \"b\"\n"),
        ("crates/b/src/lib.rs", "// b\n"),
        ("crates/c/src/lib.rs", "// c\n"),
    ] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crates"]);
}

#[tokio::test]
async fn a_cross_task_item_lands_both_tasks_files_that_each_task_forbids_the_other() {
    let mut f = Fixture::new();
    two_crates(&f);
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task(
                "TASK-001",
                "crates/a/src/lib.rs",
                &[
                    "`crates/b/src/lib.rs` (TASK-002 scope)",
                    "`crates/c/src/lib.rs` (frozen)",
                ],
            ),
            task(
                "TASK-002",
                "crates/b/src/lib.rs",
                &["`crates/a/src/lib.rs` (TASK-001 scope)"],
            ),
        ],
    });
    f.item_task_ids = vec!["TASK-001".into(), "TASK-002".into()];
    let (out, prompts) = f
        .wave_audited(
            "cross",
            vec![(
                vec!["crates/a/src/lib.rs", "crates/b/src/lib.rs"],
                Edits {
                    files: vec![
                        ("crates/a/src/lib.rs", "// a fixed\n"),
                        ("crates/b/src/lib.rs", "// b fixed\n"),
                    ],
                    report: vec!["crates/a/src/lib.rs", "crates/b/src/lib.rs"],
                    via_adapter: true,
                },
            )],
            None,
        )
        .await;
    let result = f.branch_result("cross", "cross-0");
    assert_eq!(
        result.status,
        WorkflowV2Status::Accepted,
        "{out:#?}\n{result:#?}"
    );
    // Landed: this is the marker the prelude reads to dispatch the verifier.
    assert_eq!(result.data["patch_landed"], json!(true), "{result:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/a/src/lib.rs"]),
        "// a fixed"
    );
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/b/src/lib.rs"]),
        "// b fixed"
    );
    assert!(
        result
            .residual_gaps
            .iter()
            .all(|gap| !gap.id.starts_with("forbidden")),
        "{result:#?}"
    );
    // The item is told only what it may not touch: the frozen third crate.
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].contains("declared targets take precedence): crates/c/src/lib.rs."),
        "{}",
        prompts[0]
    );
}

/// A sibling's forbidden DIRECTORY is not unfrozen by one declared file in
/// it: the declared file is the item's own, a new file beside it is not.
#[tokio::test]
async fn a_sibling_directory_stays_frozen_around_the_declared_file() {
    let mut f = Fixture::new();
    two_crates(&f);
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task(
                "TASK-001",
                "crates/a/src/lib.rs",
                &["`crates/b/` (TASK-002 scope)"],
            ),
            task("TASK-002", "crates/b/src/lib.rs", &[]),
        ],
    });
    f.item_task_ids = vec!["TASK-001".into(), "TASK-002".into()];
    let declared_only = Edits {
        files: vec![
            ("crates/a/src/lib.rs", "// a fixed\n"),
            ("crates/b/src/lib.rs", "// b fixed\n"),
        ],
        report: vec!["crates/a/src/lib.rs", "crates/b/src/lib.rs"],
        via_adapter: true,
    };
    let (_, prompts) = f
        .wave_audited(
            "own",
            vec![(
                vec!["crates/a/src/lib.rs", "crates/b/src/lib.rs"],
                declared_only,
            )],
            None,
        )
        .await;
    let landed = f.branch_result("own", "own-0");
    assert_eq!(landed.status, WorkflowV2Status::Accepted, "{landed:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/b/src/lib.rs"]),
        "// b fixed"
    );
    assert!(
        prompts[0].contains("declared targets take precedence): crates/b/."),
        "{}",
        prompts[0]
    );
    let beside = Edits {
        files: vec![
            ("crates/b/src/lib.rs", "// b again\n"),
            ("crates/b/src/helper.rs", "// new\n"),
        ],
        report: vec!["crates/b/src/lib.rs", "crates/b/src/helper.rs"],
        via_adapter: true,
    };
    f.wave_audited(
        "beside",
        vec![(vec!["crates/a/src/lib.rs", "crates/b/src/lib.rs"], beside)],
        None,
    )
    .await;
    let rejected = f.branch_result("beside", "beside-0");
    assert_eq!(
        rejected.status,
        WorkflowV2Status::NeedsReview,
        "{rejected:#?}"
    );
    assert_eq!(
        rejected.data["forbidden_paths_changed"],
        json!(["crates/b/src/helper.rs"])
    );
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/b/src/lib.rs"]),
        "// b fixed"
    );
}
