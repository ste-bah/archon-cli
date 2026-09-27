//! Issue-121 follow-up: a build side effect in another task's file is asked
//! back in the same session, not left for the landing to refuse.
//!
//! The owner rule refuses a patch that changes a file another task
//! declares. A branch's own build can regenerate one (a lockfile, a
//! manifest) without the agent ever reporting it; the landing then refuses
//! the whole patch after the session that could undo it has ended. The host
//! cannot tell a regeneration from a change the task needs, so it never
//! restores one itself: the adapter refuses the accepted result with the
//! ownership repair class, naming each such path, its holder and how to
//! restore it. A real Git worktree and the production adapter.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::*;
use serde_json::json;
use support::git;

const OWN: &str = "crates/a/src/lib.rs";
const LOCK: &str = "Cargo.lock";

fn repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path();
    git(repo, &["init", "-q"]);
    git(repo, &["config", "user.name", "fixture"]);
    git(repo, &["config", "user.email", "fixture@example.invalid"]);
    for (path, content) in [(OWN, "// a\n"), (LOCK, "# lock v1\n"), ("README", "r\n")] {
        let target = repo.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "base"]);
    temp
}

fn request(root: &std::path::Path, mode: WorkflowV2WriteMode) -> WorkflowV2AgentRequest {
    let mut call = WorkflowV2HostCall {
        id: "owner-0".into(),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: Some(mode),
        options: Default::default(),
    };
    call.options.extra.insert(
        "wave_claims".into(),
        json!([
            {"item_id": "owner-0", "owned": [OWN]},
            {"item_id": "task-owner:TASK-002", "owned": [LOCK]},
            {"item_id": "sibling-1", "owned": ["crates/b"]},
        ]),
    );
    WorkflowV2AgentRequest {
        call,
        role: "coder".into(),
        task: "Implement TASK-001".into(),
        constraints: Vec::new(),
        input: json!({}),
        repository_root: Some(root.display().to_string()),
        project_artifacts: Default::default(),
        target_files: vec![OWN.into()],
        target_ownership_scopes: Vec::new(),
    }
}

fn reporting_own_file() -> String {
    let mut result = WorkflowV2Result::accepted("implemented");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "changed the file",
    ));
    result.files_changed = vec![WorkflowV2FileRecord::new(OWN)];
    serde_json::to_string(&result).unwrap()
}

#[test]
fn an_unreported_change_to_another_holders_file_is_asked_back_with_how_to_restore_it() {
    let temp = repo();
    let root = temp.path();
    let adapter = WorkflowV2AgentAdapter::new();
    std::fs::write(root.join(OWN), "// a fixed\n").unwrap();
    // The build regenerated the lockfile another task declares, and made a
    // file in a sibling item's directory scope.
    std::fs::write(root.join(LOCK), "# lock v2\n").unwrap();
    std::fs::create_dir_all(root.join("crates/b")).unwrap();
    std::fs::write(root.join("crates/b/gen.rs"), "// generated\n").unwrap();
    let err = adapter
        .parse_agent_output(
            &request(root, WorkflowV2WriteMode::Worktree),
            &reporting_own_file(),
        )
        .expect_err("the side effect is asked back");
    let WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(message) = err else {
        panic!("not the ownership repair class: {err}");
    };
    assert!(
        message
            .contains("Cargo.lock (declared by task TASK-002, which this branch does not serve)")
            && message.contains("crates/b/gen.rs (declared by wave item sibling-1)")
            && message.contains("`git checkout -- <path>`")
            && message.contains("record a residual_gaps entry naming the file and its owner"),
        "{message}"
    );
    // Restored, the same result is accepted.
    git(root, &["checkout", "--", LOCK]);
    std::fs::remove_file(root.join("crates/b/gen.rs")).unwrap();
    adapter
        .parse_agent_output(
            &request(root, WorkflowV2WriteMode::Worktree),
            &reporting_own_file(),
        )
        .expect("nothing of anyone else's is changed");
}

#[test]
fn owned_unclaimed_and_coordinated_changes_are_not_asked_back() {
    let temp = repo();
    let root = temp.path();
    let adapter = WorkflowV2AgentAdapter::new();
    std::fs::write(root.join(OWN), "// a fixed\n").unwrap();
    // A file no holder declares: the landing grants it.
    std::fs::write(root.join("README"), "r2\n").unwrap();
    adapter
        .parse_agent_output(
            &request(root, WorkflowV2WriteMode::Worktree),
            &reporting_own_file(),
        )
        .expect("an unclaimed change is the grant's to keep");
    // The lockfile, when the write layer's widened set declares it for this
    // branch, is the branch's own.
    std::fs::write(root.join(LOCK), "# lock v2\n").unwrap();
    let mut widened = request(root, WorkflowV2WriteMode::Worktree);
    widened.input = json!({ agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY: [OWN, LOCK] });
    adapter
        .parse_agent_output(&widened, &reporting_own_file())
        .expect("a widened target is the branch's own");
    // A coordinated tree is shared: its status is not this branch's.
    adapter
        .parse_agent_output(
            &request(root, WorkflowV2WriteMode::Coordinated),
            &reporting_own_file(),
        )
        .expect("coordinated mode is not scanned");
}
