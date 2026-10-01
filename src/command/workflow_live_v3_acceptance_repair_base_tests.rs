//! A5 in a real round: a check repaired mid-round is probed on the run's
//! base commit, in a hermetic copy, and a repair that passes there goes back
//! to its author before it may be republished.

use super::super::repair_tests::{run_fixture_with, stage};
use crate::command::workflow_task_set::executability::tests::{CRASHING, FIXED};
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};

/// Passes on any tree: it reads nothing.
const VACUOUS: &str = "python3 -c 'import sys; sys.exit(0)'";

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[tokio::test]
async fn a_mid_round_repair_that_passes_on_the_base_commit_goes_back_to_its_author() {
    let run = run_fixture_with(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", CRASHING, true),
    ]);
    // The run's base: a commit before `present` was built.
    let project = run.set.project.path();
    git(project, &["init", "-q"]);
    git(project, &["config", "user.email", "test@example.invalid"]);
    git(project, &["config", "user.name", "test"]);
    git(project, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let base = git(project, &["rev-parse", "HEAD"]);
    let events = run.store.run_dir(&run.run_id).join("events.jsonl");
    let mut text = std::fs::read_to_string(&events).unwrap_or_default();
    text.push_str(
        &serde_json::json!({"detail": {"event": "repository_bound", "head": base}}).to_string(),
    );
    text.push('\n');
    std::fs::write(&events, text).unwrap();
    let client = ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, if attempt == 1 { VACUOUS } else { FIXED }),
        |_, _| true,
    );
    let (_, record) = stage(&run, &client).await;
    let prompts = client.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2, "the vacuous repair cost its attempt");
    assert!(
        prompts[1].contains("passed on the pre-implementation tree at")
            && prompts[1].contains(&base[..12]),
        "{}",
        prompts[1]
    );
    assert!(
        record.contract_repairs[0].repaired,
        "{:?}",
        record.contract_repairs
    );
    // The original crashed: its repair's pass on the round's tree is the
    // first verdict there, not a weakening.
    assert!(record.failing_checks().is_empty(), "{:?}", record.checks);
    assert_eq!(record.passed_check_ids(), vec!["AC-F-001", "AC-F-002"]);
    assert!(!project.join("present.archon-mutated").exists());
    assert_eq!(
        git(project, &["status", "--porcelain", "--", "present"]),
        "?? present"
    );
}
