//! ACC-A9 over the real acceptance stage: the run-end observation runs before
//! the terminal commit, a failed one re-enters acceptance in the same session
//! and the committed status is the one the re-entered round and the final
//! observation leave; a passing observation commits as before.

use std::sync::Mutex;

use super::*;
use archon_workflow::{
    FinalizationRecordV1, ObserverAuthority, RunEndObserverOutcomeV1, RunEndObserverStateV1,
    RunStatus, WorkflowError,
};

use super::super::WorkflowV2ScriptRuntime;
use super::super::workflow_live_v2_finalizer::{
    FINALIZATION_RECORD_PATH, RunEndObserverContext, WorkflowRunEndObserver,
};

#[path = "workflow_live_v3_run_end_heal_fixture.rs"]
mod fixture;
use fixture::{Fixture, fixture, in_run_round, snapshot};

/// R6's run-end failure, as the observer raised it.
const CHAIN_MOVED: &str = "native observer chain differs from launch pin";

/// The reason the record keeps: the observer's error as displayed.
fn chain_moved() -> String {
    WorkflowError::ArtifactInvalid(CHAIN_MOVED.into()).to_string()
}

/// Fails with `CHAIN_MOVED` for its first `failures` observations, then
/// passes. Every observation must precede this finalization's terminal
/// commit: no terminal event beyond the `committed` ones already recorded.
struct Scripted {
    store: WorkflowStore,
    failures: usize,
    /// The failing observations complete with REQ-1 failing instead.
    findings: bool,
    committed: usize,
    seen: Mutex<Vec<WorkflowV2Status>>,
}

impl WorkflowRunEndObserver for Scripted {
    fn observe(
        &self,
        context: &RunEndObserverContext<'_>,
    ) -> archon_workflow::WorkflowResult<RunEndObserverOutcomeV1> {
        assert!(context.pre_commit, "observed after the commit");
        let terminal = labels_of(&self.store, context.run_id)
            .iter()
            .filter(|label| *label == "terminal_status")
            .count();
        assert_eq!(
            terminal, self.committed,
            "observed after the terminal event"
        );
        if self.committed == 0 {
            let path = self
                .store
                .run_dir(context.run_id)
                .join(FINALIZATION_RECORD_PATH);
            assert!(!path.exists(), "a terminal record exists while observing");
            let status = self.store.load_state(context.run_id).unwrap().status;
            assert!(
                !matches!(status, RunStatus::Completed | RunStatus::NeedsReview),
                "terminal status {status:?} committed before the observation"
            );
        }
        let mut seen = self.seen.lock().unwrap();
        seen.push(context.terminal_status);
        if seen.len() <= self.failures && !self.findings {
            return Err(WorkflowError::ArtifactInvalid(CHAIN_MOVED.into()));
        }
        let failing = seen.len() <= self.failures;
        if failing {
            let row = r#"{"record_kind":"policy_shadow","acceptance_id":"REQ-1"}"#;
            let path = super::super::workflow_run_end_observer::RUN_END_OBSERVER_RECORDS_PATH;
            self.store
                .write_run_file(context.run_id, path, format!("{row}\n").as_bytes())
                .unwrap();
        }
        Ok(RunEndObserverOutcomeV1 {
            authority: ObserverAuthority::ObserveOnly,
            evaluated_floor_count: 1,
            policy_finding_count: usize::from(failing),
            operational_deferral_count: 0,
        })
    }
}

fn scripted(fixture: &Fixture, failures: usize) -> Scripted {
    Scripted {
        store: fixture.store.clone(),
        failures,
        findings: false,
        committed: labels(fixture)
            .iter()
            .filter(|label| *label == "terminal_status")
            .count(),
        seen: Mutex::new(Vec::new()),
    }
}

async fn finalize(fixture: &Fixture, observer: &Scripted) -> WorkflowV2ScriptSummary {
    let summary = in_run_round(fixture).await;
    finalize_with(fixture, observer, summary).await
}

async fn finalize_with(
    fixture: &Fixture,
    observer: &Scripted,
    summary: WorkflowV2ScriptSummary,
) -> WorkflowV2ScriptSummary {
    try_finalize_with(fixture, observer, summary)
        .await
        .expect("finalizes")
}

async fn try_finalize_with(
    fixture: &Fixture,
    observer: &Scripted,
    summary: WorkflowV2ScriptSummary,
) -> anyhow::Result<WorkflowV2ScriptSummary> {
    finalize_run_observed(
        &fixture.store,
        &fixture.run_id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        Some(snapshot(fixture)),
        summary,
        &fixture.v2_store,
        observer,
        Some((&fixture.runtime, None, Some(&fixture.universe))),
        None,
    )
    .await
}

/// Finalization that must stop on a stall: the run is paused with the
/// evidence (Issue 262), and the message is returned.
async fn finalize_paused(fixture: &Fixture, observer: &Scripted) -> String {
    let summary = in_run_round(fixture).await;
    let error = try_finalize_with(fixture, observer, summary)
        .await
        .expect_err("a stall pauses the run, never ends it");
    match error
        .chain()
        .find_map(|cause| cause.downcast_ref::<WorkflowError>())
    {
        Some(WorkflowError::ControlPaused(message)) => {
            assert_eq!(
                fixture.store.load_state(&fixture.run_id).unwrap().status,
                RunStatus::Paused
            );
            message.clone()
        }
        _ => panic!("a stall pauses the run, never ends it: {error:#}"),
    }
}

fn record(fixture: &Fixture) -> FinalizationRecordV1 {
    let path = fixture
        .store
        .run_dir(&fixture.run_id)
        .join(FINALIZATION_RECORD_PATH);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn labels(fixture: &Fixture) -> Vec<String> {
    labels_of(&fixture.store, &fixture.run_id)
}

fn labels_of(store: &WorkflowStore, run_id: &str) -> Vec<String> {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .map(|event| {
            event["detail"]["event"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

fn position(labels: &[String], label: &str) -> usize {
    labels
        .iter()
        .position(|seen| seen == label)
        .unwrap_or_else(|| panic!("no {label}: {labels:?}"))
}

/// The deliverable was lost after the in-run round passed. The observation
/// fails before the commit, acceptance is re-entered, its round fails REQ-1,
/// and the committed status is that round's `NeedsReview`, never `Completed`.
#[tokio::test]
async fn run_end_observer_failure_before_commit_reopens_acceptance_and_commits_its_result() {
    let fixture = fixture();
    let observer = scripted(&fixture, 1);
    let summary = in_run_round(&fixture).await;
    std::fs::remove_file(fixture.repo.path().join("present")).unwrap();
    let finalized = finalize_with(&fixture, &observer, summary).await;
    assert_eq!(finalized.status, WorkflowV2Status::NeedsReview);
    let next = finalized.next_action.expect("names the failing check");
    assert!(
        next.contains("REQ-1") && next.contains("TASK-H-001"),
        "{next}"
    );
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::NeedsReview
    );
    assert_eq!(
        *observer.seen.lock().unwrap(),
        vec![WorkflowV2Status::Accepted, WorkflowV2Status::NeedsReview],
        "observed again on the re-decided outcome"
    );
    let record = record(&fixture);
    assert_eq!(record.terminal_status, RunStatus::NeedsReview);
    let gate = record.acceptance_gate.expect("the re-entered round's gate");
    assert_eq!(gate.record_path, "v2/acceptance/round-01/attempt-02.json");
    assert_eq!(gate.failing_check_ids, vec!["REQ-1"]);
    assert_eq!(record.prior_observer_failures, vec![chain_moved()]);
    assert!(matches!(
        record.observer_state,
        Some(RunEndObserverStateV1::Completed { .. })
    ));
    let labels = labels(&fixture);
    assert!(
        position(&labels, "run_end_acceptance_reopened") < position(&labels, "terminal_status")
    );
}

/// A failure the re-entered round clears heals: the round passes again, the
/// next observation passes, and the run completes on that round.
#[tokio::test]
async fn run_end_observer_failure_that_reopened_acceptance_clears_heals_to_completion() {
    let fixture = fixture();
    let observer = scripted(&fixture, 1);
    let finalized = finalize(&fixture, &observer).await;
    assert_eq!(finalized.status, WorkflowV2Status::Accepted);
    assert!(finalized.next_action.is_none());
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(observer.seen.lock().unwrap().len(), 2);
    let record = record(&fixture);
    assert_eq!(
        record.acceptance_gate.expect("gate").record_path,
        "v2/acceptance/round-01/attempt-02.json"
    );
    assert_eq!(record.prior_observer_failures, vec![chain_moved()]);
    assert!(matches!(
        record.observer_state,
        Some(RunEndObserverStateV1::Completed { .. })
    ));
}

/// Re-entry follows progress: the re-entered round decides the same outcome
/// on the same pin and the observation fails the same way, so it stops; the
/// standing failure pauses the run with the evidence (Issue 262), never
/// `NeedsReview`.
#[tokio::test]
async fn run_end_observer_failure_that_makes_no_progress_pauses_the_run() {
    let fixture = fixture();
    let observer = scripted(&fixture, usize::MAX);
    let message = finalize_paused(&fixture, &observer).await;
    assert!(
        message.contains(CHAIN_MOVED) && message.contains("made no progress"),
        "{message}"
    );
    assert_eq!(
        observer.seen.lock().unwrap().len(),
        2,
        "one re-entry, then no progress"
    );
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    assert!(
        run_dir
            .join("v2/acceptance/round-01/attempt-02.json")
            .is_file()
    );
    assert!(
        !run_dir
            .join("v2/acceptance/round-01/attempt-03.json")
            .exists()
    );
    assert!(
        labels(&fixture)
            .iter()
            .any(|label| label == "run_end_acceptance_observer_stall_pause"),
        "{:?}",
        labels(&fixture)
    );
}

/// A passing observation commits exactly as before: once, before the
/// commit, with no re-entry.
#[tokio::test]
async fn run_end_observer_pass_before_commit_finalizes_as_before() {
    let fixture = fixture();
    let observer = scripted(&fixture, 0);
    let finalized = finalize(&fixture, &observer).await;
    assert_eq!(finalized.status, WorkflowV2Status::Accepted);
    assert_eq!(
        fixture.store.load_state(&fixture.run_id).unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(
        *observer.seen.lock().unwrap(),
        vec![WorkflowV2Status::Accepted]
    );
    let record = record(&fixture);
    assert!(record.terminal_event_committed);
    assert!(record.prior_observer_failures.is_empty());
    assert_eq!(
        record.acceptance_gate.expect("gate").record_path,
        "v2/acceptance/round-01/attempt-01.json"
    );
    assert!(matches!(
        record.observer_state,
        Some(RunEndObserverStateV1::Completed { .. })
    ));
    assert!(
        !fixture
            .store
            .run_dir(&fixture.run_id)
            .join("v2/acceptance/round-01/attempt-02.json")
            .exists()
    );
    let labels = labels(&fixture);
    assert!(
        position(&labels, "run_end_acceptance_observer_started")
            < position(&labels, "terminal_status")
    );
}

/// A run the observation left `NeedsReview` resumes: its acceptance runs a
/// new round that still fails, and that round's outcome supersedes the
/// committed one instead of being refused as a changed replay.
#[tokio::test]
async fn a_blocked_run_resumed_onto_a_new_failing_round_commits_that_round() {
    let fixture = fixture();
    let summary = in_run_round(&fixture).await;
    std::fs::remove_file(fixture.repo.path().join("present")).unwrap();
    let first = finalize_with(&fixture, &scripted(&fixture, 1), summary).await;
    assert_eq!(first.status, WorkflowV2Status::NeedsReview);
    let resumed = in_run_round(&fixture).await;
    let observer = scripted(&fixture, 0);
    let finalized = finalize_with(&fixture, &observer, resumed).await;
    assert_eq!(finalized.status, WorkflowV2Status::NeedsReview);
    assert_eq!(observer.seen.lock().unwrap().len(), 1);
    let record = record(&fixture);
    assert_eq!(
        record.acceptance_gate.expect("gate").record_path,
        "v2/acceptance/round-01/attempt-03.json"
    );
    assert!(record.terminal_event_committed);
    let terminal = labels(&fixture)
        .iter()
        .filter(|label| *label == "terminal_status")
        .count();
    assert_eq!(terminal, 2, "the new outcome is committed once");
}

/// B2: an observation that completes with a failing check never commits
/// `Accepted`; it re-opens acceptance, and once the round clears and the
/// next observation is clean, the run completes on that round.
#[tokio::test]
async fn run_end_observation_findings_reopen_acceptance_and_heal_when_cleared() {
    let fixture = fixture();
    let observer = Scripted {
        findings: true,
        ..scripted(&fixture, 1)
    };
    let finalized = finalize(&fixture, &observer).await;
    assert_eq!(finalized.status, WorkflowV2Status::Accepted);
    assert_eq!(observer.seen.lock().unwrap().len(), 2, "re-observed once");
    let record = record(&fixture);
    assert_eq!(
        record.acceptance_gate.expect("gate").record_path,
        "v2/acceptance/round-01/attempt-02.json"
    );
    let failures = &record.prior_observer_failures;
    assert!(
        failures.len() == 1 && failures[0].contains("REQ-1"),
        "{failures:?}"
    );
}

/// B2 / Issue 262: findings that still stand once re-entry stops making
/// progress never commit `Accepted`: the run is paused, naming the check.
#[tokio::test]
async fn run_end_observation_findings_never_commit_accepted_and_pause_naming_the_check() {
    let fixture = fixture();
    let observer = Scripted {
        findings: true,
        ..scripted(&fixture, usize::MAX)
    };
    let message = finalize_paused(&fixture, &observer).await;
    assert!(
        message.contains("failing frozen check") && message.contains("REQ-1"),
        "{message}"
    );
    let run_dir = fixture.store.run_dir(&fixture.run_id);
    assert!(
        run_dir
            .join("v2/acceptance/round-01/attempt-02.json")
            .is_file()
    );
}

#[path = "workflow_live_v3_run_end_call_heal_tests.rs"]
mod call_heal;
#[path = "workflow_live_v3_run_end_record_heal_tests.rs"]
mod record_heal;
