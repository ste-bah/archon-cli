//! Issue-73: a baseline failure the coder cannot be held to — one whose file
//! is forbidden to its task, one no file can be resolved for — is listed to
//! ignore and recorded unowned, never made the task's to make pass.

use archon_write_plan::ForbiddenPaths;

use super::super::test_baseline::tests::{Host, head, repository};
use super::super::test_baseline_preamble::preamble;
use super::{BranchBaselineRequest, WaveBaselineContext, establish_wave};
use crate::v2::WorkflowV2ResultStore;

/// A command shaped like a cargo run of package `app` that reports `tests`
/// as failed and exits red.
fn red(command: &str, tests: &[&str]) -> String {
    let lines: String = tests
        .iter()
        .map(|test| format!("test {test} ... FAILED\\n"))
        .collect();
    format!(": {command} ; printf '{lines}'; exit 101")
}

fn request(command: &str, forbidden: &[&str], worktree: &std::path::Path) -> BranchBaselineRequest {
    BranchBaselineRequest {
        branch_id: "agents-3-a".into(),
        task_ids: vec!["TASK-A".into()],
        commands: vec![command.to_string()],
        worktree: worktree.to_path_buf(),
        targets: vec!["src/mine.rs".into()],
        forbidden: ForbiddenPaths::from_entries(forbidden.iter()),
    }
}

#[tokio::test]
async fn an_integration_failure_lands_on_its_test_file_and_is_ignored_when_that_file_is_forbidden()
{
    let temp = tempfile::tempdir().unwrap();
    let (canonical, ws) = repository(temp.path());
    std::fs::create_dir_all(ws.join("tests")).unwrap();
    std::fs::write(ws.join("tests/gates.rs"), "// integration\n").unwrap();
    let base = head(&canonical);
    let command = red(
        "cargo nextest run -p app --test gates",
        &["current_artifact_integrity_is_required"],
    );

    // Nobody forbids the file: it is the task's, and it is the TEST file —
    // not `src/lib.rs`, which is what the crate-root fallback used to give.
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let ctx = WaveBaselineContext {
        store: &store,
        dispatch: &Host,
        universe: None,
        stage_id: "agents-3",
        base_commit: &base,
        parallelism: 1,
    };
    let records = establish_wave(&ctx, &[request(&command, &[], &ws)]).await;
    assert_eq!(records[0].obligation_files(), vec!["tests/gates.rs"]);
    assert!(
        !format!("{:?}", records[0]).contains("src/lib.rs"),
        "{:?}",
        records[0]
    );

    // Forbidden: ignored and unowned, never an obligation, and the coder is
    // told to ignore it.
    let store = WorkflowV2ResultStore::new(temp.path().join("run2/v2"));
    let ctx = WaveBaselineContext {
        store: &store,
        ..ctx
    };
    let records = establish_wave(&ctx, &[request(&command, &["tests/gates.rs"], &ws)]).await;
    let record = &records[0];
    assert!(record.obligations.is_empty(), "{record:?}");
    assert!(record.must_pass().is_empty(), "{record:?}");
    assert!(record.obligation_files().is_empty(), "{record:?}");
    assert!(record.routed.is_empty(), "{record:?}");
    assert_eq!(record.ignored.len(), 1, "{record:?}");
    assert_eq!(
        record.ignored[0].test_id,
        "current_artifact_integrity_is_required"
    );
    assert_eq!(record.ignored[0].file.as_deref(), Some("tests/gates.rs"));
    let text = preamble(record);
    assert!(
        text.contains(
            "ignore them and leave their files alone: \
                       current_artifact_integrity_is_required (tests/gates.rs;"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn a_test_id_no_file_can_be_resolved_for_is_unowned_and_not_this_tasks_obligation() {
    let temp = tempfile::tempdir().unwrap();
    let (canonical, ws) = repository(temp.path());
    let base = head(&canonical);
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let ctx = WaveBaselineContext {
        store: &store,
        dispatch: &Host,
        universe: None,
        stage_id: "agents-3",
        base_commit: &base,
        parallelism: 1,
    };
    let command = red("cargo test -p app", &["nowhere::at_all", "bare_id"]);
    let records = establish_wave(&ctx, &[request(&command, &[], &ws)]).await;
    let record = &records[0];
    assert_eq!(record.commands[0].failing_tests.len(), 2);
    assert!(record.obligations.is_empty(), "{record:?}");
    assert!(record.must_pass().is_empty(), "{record:?}");
    assert!(record.routed.is_empty(), "{record:?}");
    let ignored: Vec<&str> = record
        .ignored
        .iter()
        .map(|failure| failure.test_id.as_str())
        .collect();
    assert_eq!(ignored, vec!["bare_id", "nowhere::at_all"]);
    assert!(record.ignored.iter().all(|failure| failure.file.is_none()));
    let text = preamble(record);
    assert!(
        text.contains(
            "ignore them and leave their files alone: bare_id (no file could be \
                       resolved from its test id)"
        ),
        "{text}"
    );
}
