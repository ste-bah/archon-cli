//! Batch L (L1) at the acceptance stage: a round never runs on a tree that
//! holds a remediation its verifier refused. The refused landing here is
//! exactly what makes a check pass; the round takes it back out first, so
//! the check fails honestly, and a landing that cannot be taken back out
//! holds the round as a HIGH operational finding.

use super::tests::{Fixture, execution, failing_ids, fixture_with, run};
use archon_workflow::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2WriteMode,
};

const FIX: &str = "review-remediate-task-f-002-1-1";
const VERDICT: &str = "verification-wave-review-verify-task-f-002-1-2";

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
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

fn record(
    fixture: &Fixture,
    id: &str,
    stage: &str,
    status: WorkflowV2Status,
) -> WorkflowV2CallRecord {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        serde_json::json!({"version": 1, "stage": stage, "taskId": "TASK-F-002", "round": 1,
            "maxRounds": 1, "sourceReduceCallIds": ["acceptance"],
            "observedBy": ["acceptance-contract-run-1"]}),
    );
    let fix = stage == "remediate";
    let call = WorkflowV2HostCall {
        id: id.into(),
        method: if fix {
            WorkflowV2HostMethod::Fanout
        } else {
            WorkflowV2HostMethod::Parallel
        },
        write_mode: fix.then_some(WorkflowV2WriteMode::Worktree),
        options,
    };
    let mut result = WorkflowV2Result::accepted("answered");
    result.status = status;
    result.summary = format!("{id}: the entry it registers has no data behind it");
    WorkflowV2CallRecord::new(
        fixture.run_id.clone(),
        call,
        1,
        "input".into(),
        result,
        vec![],
    )
}

/// A remediation of TASK-F-002 lands the file REQ-2 checks for, and its
/// verifier refuses it.
fn refused_landing(fixture: &Fixture) -> WorkflowV2ResultStore {
    let repo = fixture.repo.path();
    std::fs::write(repo.join("missing"), "made to pass\n").unwrap();
    git(repo, &["add", "missing"]);
    git(
        repo,
        &[
            "-c",
            "user.name=archon-workflow",
            "-c",
            "user.email=archon-workflow@local",
            "commit",
            "-qm",
            &format!(
                "archon: wave 0 outputs (run {}, stage {FIX})",
                fixture.run_id
            ),
        ],
    );
    let store = WorkflowV2ResultStore::new(fixture.store.run_dir(&fixture.run_id).join("v2"));
    store
        .save_call_record(&record(
            fixture,
            FIX,
            "remediate",
            WorkflowV2Status::Accepted,
        ))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    store
        .save_call_record(&record(
            fixture,
            VERDICT,
            "verify",
            WorkflowV2Status::NeedsReview,
        ))
        .unwrap();
    store
}

#[tokio::test]
async fn a_round_takes_a_refused_landing_out_before_it_checks_anything() {
    let fixture = fixture_with(true, "test -f missing");
    let store = refused_landing(&fixture);
    let result = run(&fixture, &execution(2, 3, &[]))
        .await
        .expect("round runs");
    assert_eq!(
        failing_ids(&result),
        vec!["REQ-2", "REQ-9"],
        "REQ-2 passed only on the refused landing: {result:#?}"
    );
    assert!(!fixture.repo.path().join("missing").exists());
    let subject = git(fixture.repo.path(), &["log", "-1", "--format=%s"]);
    assert_eq!(
        subject,
        format!(
            "archon: revert refused landing (run {}, stage {FIX})",
            fixture.run_id
        )
    );
    let log =
        archon_workflow::v2::script::refused_landings::refused_landing_reverts(store.run_root())
            .unwrap();
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(
        (log[0].outcome.as_str(), log[0].verdict_call_id.as_str()),
        ("reverted", VERDICT)
    );
    assert!(
        result.data["operational_errors"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_refused_landing_that_cannot_come_out_holds_the_round() {
    let fixture = fixture_with(true, "test -f missing");
    refused_landing(&fixture);
    // Someone's uncommitted work on the landing's path: reverting would
    // sweep it into the revert, so nothing is written.
    std::fs::write(fixture.repo.path().join("missing"), "edited by hand\n").unwrap();
    let result = run(&fixture, &execution(1, 3, &[]))
        .await
        .expect("round runs");
    // Batch O (A2): held, not final -- the next round retries the revert,
    // and only a round with no progress ends the loop.
    assert_eq!(result.data["final"], false, "{result:#?}");
    let errors = result.data["operational_errors"].as_array().unwrap();
    assert!(
        errors.iter().any(|error| {
            let error = error.as_str().unwrap_or_default();
            error.contains(FIX) && error.contains(VERDICT) && error.contains("uncommitted work")
        }),
        "{errors:#?}"
    );
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.severity.as_deref() == Some("high") && gap.description.contains(FIX))
    );
}
