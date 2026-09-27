//! Issue-120: a write branch is shown exactly the scope its landing keeps.
//!
//! Live on wf-0ddadd81 a residual round's prompt listed eleven target
//! files, its tool guard refused an Edit of an undeclared file as "would be
//! lost", and the landing then declared fifteen: the ownership grant kept
//! four undeclared files inside the scope roots that no other item of the
//! single-item wave claimed, written by a shell heredoc the guard could not
//! see. This drives the production write wave with real Git worktrees and
//! checks the prompt the branch is dispatched with names the rule and
//! carries the grant's own roots and the OTHER items' claims for the guard,
//! and that the landing keeps exactly what that stamp admits: the unclaimed
//! in-root file lands, and a file the sibling claims refuses the branch.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::{Value, json};
use support::{Edits, Fixture, git};

const OWN: &str = "crates/a/src/lib.rs";
const SIBLING: &str = "crates/a/src/other.rs";
const UNCLAIMED: &str = "crates/a/src/extra.rs";

fn with_crate(f: &Fixture) {
    for (path, content) in [
        ("crates/a/Cargo.toml", "[package]\nname = \"a\"\n"),
        (OWN, "// a\n"),
        (SIBLING, "// other\n"),
        (UNCLAIMED, "// extra\n"),
    ] {
        let target = f.repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "crate"]);
}

fn edits(files: Vec<(&'static str, &'static str)>) -> Edits {
    Edits {
        report: files.iter().map(|(path, _)| *path).collect(),
        files,
        via_adapter: false,
    }
}

/// The `_grantable_scope` object the dispatched branch's input carried.
fn stamp_in(f: &Fixture, call_id: &str) -> Value {
    f.input_stamp(call_id, "_grantable_scope")
}

/// The stamps are read from the input by the dispatch; never shown.
fn assert_unrendered(prompt: &str) {
    for key in [
        "_grantable_scope",
        "_declared_targets",
        "_forbidden_paths",
        "_isolated_worktree",
    ] {
        assert!(!prompt.contains(key), "{key} rendered: {prompt}");
    }
}

#[tokio::test]
async fn the_branch_is_told_and_stamped_the_scope_its_landing_keeps() {
    let f = Fixture::new();
    with_crate(&f);
    let (_, prompts) = f
        .wave_audited(
            "scope",
            vec![
                (
                    vec![OWN],
                    edits(vec![(OWN, "// a fixed\n"), (UNCLAIMED, "// extra fixed\n")]),
                ),
                (vec![SIBLING], edits(vec![(SIBLING, "// other fixed\n")])),
            ],
            None,
        )
        .await;
    let own = prompts
        .iter()
        .find(|prompt| prompt.contains("call_id: scope-0"))
        .unwrap_or_else(|| panic!("{prompts:#?}"));
    assert_unrendered(own);
    assert!(
        own.contains("Your declared scope is target_files plus every file, new or existing"),
        "{own}"
    );
    assert!(
        own.contains(
            "that no other item of this wave declares is granted to this branch at landing"
        ),
        "{own}"
    );
    // The sibling's claim is its declared file and that module's own
    // directory scope, exactly as the grant's wave claims hold them.
    // A worktree-mode branch is marked as running in its own worktree.
    assert_eq!(f.input_stamp("scope-0", "_isolated_worktree"), json!(true));
    assert_eq!(
        stamp_in(&f, "scope-0"),
        json!({"claimed": ["crates/a/src/other", SIBLING], "scope_roots": ["crates/a/"]})
    );
    // The landing keeps what the stamp admits: the unclaimed in-root file
    // was granted and landed under branch 0.
    assert_eq!(
        git(&f.repo, &["show", &format!("HEAD:{UNCLAIMED}")]),
        "// extra fixed"
    );
    let result = f.branch_result("scope", "scope-0");
    assert_eq!(
        result.data["scope_granted"],
        json!([UNCLAIMED]),
        "{result:#?}"
    );
}

#[tokio::test]
async fn a_file_the_stamp_names_as_another_items_claim_refuses_the_branch_at_landing() {
    let f = Fixture::new();
    with_crate(&f);
    let (_, prompts) = f
        .wave_audited(
            "scope",
            vec![
                (
                    vec![OWN],
                    edits(vec![(OWN, "// a fixed\n"), (SIBLING, "// other, by 0\n")]),
                ),
                (vec![SIBLING], edits(vec![(SIBLING, "// other fixed\n")])),
            ],
            None,
        )
        .await;
    // The branch was stamped the claim the landing then refused it over.
    let own = prompts
        .iter()
        .find(|prompt| prompt.contains("call_id: scope-0"))
        .unwrap_or_else(|| panic!("{prompts:#?}"));
    assert_unrendered(own);
    assert!(
        stamp_in(&f, "scope-0")["claimed"]
            .as_array()
            .unwrap()
            .contains(&json!(SIBLING)),
        "{own}"
    );
    let result = f.branch_result("scope", "scope-0");
    assert_ne!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(git(&f.repo, &["show", &format!("HEAD:{OWN}")]), "// a");
}

/// Issue-121: the universe the owner rule reads: TASK-001 (the wave's one
/// item) declares its own file and one more, TASK-002 declares `SIBLING`.
fn owners(f: &mut Fixture, own_extra: &str) {
    use archon_workflow::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
    let task = |id: &str, files: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        files_expected_to_change: files.iter().map(|f| (*f).to_string()).collect(),
        ..Default::default()
    };
    f.universe = Some(WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-001", &[OWN, own_extra]),
            task("TASK-002", &[SIBLING]),
        ],
    });
}

#[tokio::test]
async fn a_single_item_round_writing_another_tasks_file_is_refused_at_guard_and_landing() {
    let mut f = Fixture::new();
    with_crate(&f);
    owners(&mut f, UNCLAIMED);
    let (_, prompts) = f
        .wave_audited(
            "owner",
            vec![(
                vec![OWN],
                edits(vec![(OWN, "// a fixed\n"), (SIBLING, "// other, by 0\n")]),
            )],
            None,
        )
        .await;
    let own = prompts
        .iter()
        .find(|prompt| prompt.contains("call_id: owner-0"))
        .unwrap_or_else(|| panic!("{prompts:#?}"));
    assert_unrendered(own);
    // No other item exists, yet the guard is stamped TASK-002's file as
    // claimed: it refuses the write when it is attempted.
    assert_eq!(
        stamp_in(&f, "owner-0"),
        json!({"claimed": [SIBLING], "scope_roots": ["crates/a/"]})
    );
    // And the landing refuses it by the same claim: nothing lands.
    let result = f.branch_result("owner", "owner-0");
    assert_ne!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(git(&f.repo, &["show", &format!("HEAD:{OWN}")]), "// a");
    assert_eq!(
        git(&f.repo, &["show", &format!("HEAD:{SIBLING}")]),
        "// other"
    );
}

#[tokio::test]
async fn an_own_tasks_undeclared_file_and_an_unowned_in_root_file_are_still_granted() {
    const OWN_EXTRA: &str = "crates/a/src/own_extra.rs";
    const NOBODYS: &str = "crates/a/src/nobodys.rs";
    let mut f = Fixture::new();
    with_crate(&f);
    owners(&mut f, OWN_EXTRA);
    let (_, prompts) = f
        .wave_audited(
            "owner",
            vec![(
                vec![OWN],
                edits(vec![
                    (OWN, "// a fixed\n"),
                    (OWN_EXTRA, "// own extra\n"),
                    (NOBODYS, "// nobody's\n"),
                ]),
            )],
            None,
        )
        .await;
    let own = prompts
        .iter()
        .find(|prompt| prompt.contains("call_id: owner-0"))
        .unwrap_or_else(|| panic!("{prompts:#?}"));
    assert_unrendered(own);
    assert_eq!(
        stamp_in(&f, "owner-0")["claimed"],
        json!([SIBLING]),
        "only the other task's file"
    );
    let result = f.branch_result("owner", "owner-0");
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    // The own task's file is the branch's (the host widens the plan to the
    // task's declared files); the file no task declares is granted.
    assert_eq!(
        result.data["scope_granted"],
        json!([NOBODYS]),
        "{result:#?}"
    );
    let declared = f.manifest("owner", "owner-0")["declared_target_files"].clone();
    assert!(
        declared.as_array().unwrap().contains(&json!(OWN_EXTRA))
            && declared.as_array().unwrap().contains(&json!(NOBODYS)),
        "{declared}"
    );
    assert_eq!(
        git(&f.repo, &["show", &format!("HEAD:{NOBODYS}")]),
        "// nobody's"
    );
    assert_eq!(
        git(&f.repo, &["show", &format!("HEAD:{OWN_EXTRA}")]),
        "// own extra"
    );
}
