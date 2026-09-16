//! Issue-30: a write branch that changes a path its task forbids is rejected
//! and lands nothing; its sibling in the same wave still lands.
//!
//! Live on wf-719ff3b0 `agents-14-1`, the task's `Files Forbidden to Change`
//! list named the crate's gate and coverage modules; the coder edited both,
//! the ownership grant admitted them as unclaimed in-scope changes, and they
//! were committed under the task. This drives the production write-wave seam
//! with real Git writes under a task universe that declares the list, and
//! proves the branch goes `needs_review` with the typed gap, captures no
//! manifest, leaves the canonical tree untouched, keeps its worktree work as
//! a partial for the next attempt, and was told the list in its preamble —
//! while the in-scope sibling branch in the same wave lands as before.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

fn universe(forbidden: &[&str]) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-001".into(),
            source_path: "tasks/TASK-001.md".into(),
            files_forbidden_to_change: forbidden.iter().map(|f| f.to_string()).collect(),
            ..Default::default()
        }],
    }
}

/// A crate marked by `Cargo.toml` holding an owned module and a forbidden
/// gate module beside it, so the gate is in scope, unclaimed and real — the
/// exact shape every earlier rule admits.
fn with_crate(f: &Fixture) {
    for (path, content) in [
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        ("crates/a/src/lib.rs", "// a\n"),
        ("crates/a/src/gate.rs", "// gate\n"),
        ("crates/a/src/other.rs", "// other\n"),
    ] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crate"]);
}

#[tokio::test]
async fn a_branch_that_edits_a_forbidden_file_is_rejected_and_its_sibling_still_lands() {
    let mut f = Fixture::new();
    with_crate(&f);
    f.universe = Some(universe(&[
        "`crates/a/src/gate.rs` (frozen; defects are reported, never fixed here)",
    ]));
    let (out, prompts) = f
        .wave_audited(
            "forbid",
            vec![
                (
                    vec!["crates/a/src/lib.rs"],
                    Edits {
                        files: vec![
                            ("crates/a/src/lib.rs", "// a implemented\n"),
                            ("crates/a/src/gate.rs", "// gate edited\n"),
                        ],
                        report: vec!["crates/a/src/lib.rs", "crates/a/src/gate.rs"],
                        via_adapter: true,
                    },
                ),
                (
                    vec!["crates/a/src/other.rs"],
                    Edits {
                        files: vec![("crates/a/src/other.rs", "// other implemented\n")],
                        report: vec!["crates/a/src/other.rs"],
                        via_adapter: true,
                    },
                ),
            ],
            None,
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    // The canonical tree: the sibling landed, the offender landed nothing.
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/a/src/other.rs"]),
        "// other implemented"
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:crates/a/src/lib.rs"]), "// a");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:crates/a/src/gate.rs"]),
        "// gate"
    );
    assert_eq!(git(&f.repo, &["status", "--porcelain", "--", "crates"]), "");
    let sibling = f.branch_result("forbid", "forbid-1");
    assert_eq!(sibling.status, WorkflowV2Status::Accepted, "{sibling:#?}");
    assert_eq!(sibling.data["patch_landed"], json!(true));
    assert_eq!(
        f.manifest("forbid", "forbid-1")["changed_files"],
        json!(["crates/a/src/other.rs"])
    );

    let result = f.branch_result("forbid", "forbid-0");
    assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
    assert_eq!(
        result.summary,
        "write item 'forbid-0' changed 1 path(s) the task forbids: crates/a/src/gate.rs; the \
         patch was not captured"
    );
    let gap = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id == "forbidden_path_changed_forbid-0")
        .unwrap_or_else(|| panic!("no forbidden gap: {result:#?}"));
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(gap.description.contains("crates/a/src/gate.rs"), "{gap:?}");
    assert!(gap.description.contains("nothing landed"), "{gap:?}");
    assert_eq!(result.data["failure_kind"], json!("semantic"));
    assert_eq!(
        result.data["forbidden_paths_changed"],
        json!(["crates/a/src/gate.rs"])
    );
    assert_eq!(result.data["patch_landed"], json!(false), "{result:#?}");
    assert!(result.data.get("scope_granted").is_none(), "{result:#?}");
    assert!(
        !f.store
            .run_dir(&f.run)
            .join("write-coordination/stages/forbid/manifests/forbid-0.json")
            .exists(),
        "a rejected branch must capture no manifest"
    );
    // The worktree work is kept as a partial, with this verdict as its
    // origin, so the next attempt is told what to undo rather than "timed out".
    let partial = &result.data["partial_work"];
    assert_eq!(
        partial["files"],
        json!(["crates/a/src/gate.rs", "crates/a/src/lib.rs"]),
        "{result:#?}"
    );
    assert_eq!(partial["origin"]["status"], json!("needs_review"));
    assert!(
        partial["origin"]["residual_gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap["id"] == json!("forbidden_path_changed_forbid-0")),
        "{partial:#?}"
    );
    // Both branches were told the list, after the scope roots sentence.
    assert_eq!(prompts.len(), 2, "{prompts:#?}");
    for prompt in &prompts {
        let roots = prompt.find("Scope roots: crates/a/.").expect(prompt);
        let forbidden = prompt
            .find(
                "Forbidden paths for this task (never edit; a needed change there is a residual \
                 gap to report, not an edit to make): crates/a/src/gate.rs.",
            )
            .expect(prompt);
        assert!(roots < forbidden, "{prompt}");
    }
}

/// The list applies only when a forbidden path was actually CHANGED: a
/// branch whose task forbids a file it never touched lands as before, and
/// is still told the list.
#[tokio::test]
async fn an_untouched_forbidden_path_does_not_reject_the_branch() {
    let mut f = Fixture::new();
    with_crate(&f);
    f.universe = Some(universe(&["crates/a/src/gate.rs", "docs/"]));
    let (out, prompts) = f
        .wave_audited(
            "clean",
            vec![(
                vec!["crates/a/src/lib.rs"],
                Edits {
                    files: vec![("crates/a/src/lib.rs", "// a implemented\n")],
                    report: vec!["crates/a/src/lib.rs"],
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
    let result = f.branch_result("clean", "clean-0");
    assert_eq!(result.data["patch_landed"], json!(true));
    assert!(
        result.data.get("forbidden_paths_changed").is_none(),
        "{result:#?}"
    );
    assert!(
        prompts[0].contains("crates/a/src/gate.rs, docs/."),
        "{}",
        prompts[0]
    );
}
