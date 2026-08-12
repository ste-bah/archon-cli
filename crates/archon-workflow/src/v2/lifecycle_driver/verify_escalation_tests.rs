// A verification finding about the artifact must reach a writer, not be
// re-read forever.
//
// A retry re-runs a READ-ONLY verifier, which settles "the verifier just needs
// to look again" and nothing else. Observed live: three rounds of repair-plan →
// shape-repair → re-verify against a byte-identical 53KB report, each round
// spending budget on the same read, because triage filed a content critique as
// a retry item and the writer was never requested.
//
// One retry round is a fair test of that hypothesis. After it, the evidence
// says the artifact must change, so the failures escalate to write remediation.
// Which stage ran is a claim about the loop's shape, so a recording host is the
// only thing that can witness it.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::lifecycle_driver::review_test_host::RecordingHost;
use crate::v2::lifecycle_driver::{LifecycleDriver, LifecycleEvidence, LifecycleLimits};

const TASK: &str = "TASK-TDL-001";

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["tasks".to_string()],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: TASK.to_string(),
            source_path: "tasks/TASK-TDL-001.md".to_string(),
            ..WorkflowV2TaskUniverseTask::default()
        }],
    }
}

fn driver(host: Arc<RecordingHost>) -> LifecycleDriver {
    LifecycleDriver::new(
        host,
        universe(),
        None,
        None,
        Value::Null,
        std::collections::BTreeSet::new(),
        LifecycleLimits {
            max_repair_iterations: 6,
            max_investigation_iterations: 6,
            implementation_wave_max_parallelism: Some(1),
        },
    )
}

fn plan_item() -> Value {
    json!({
        "item_id": "verify-tdl-001",
        "canonical_task_ids": [TASK],
        "focused_verification": ["check the provider inventory"],
        "expected_evidence": ["each provider row addresses all eight dimensions"],
    })
}

/// The live finding: a content critique filed as a retry item.
fn triage_with_only_retry_items() -> Value {
    json!({
        "status": "accepted",
        "summary": "classified verification failures",
        "data": {
            "implementation_failures": [],
            "retry_items": [{
                "item_id": "verify-tdl-001",
                "canonical_task_ids": [TASK],
                "reason": "revise each provider row to address exact-native capability",
            }],
            "superseded_items": [],
            "terminal_blockers": [],
        },
    })
}

fn failed_verification() -> Value {
    json!({
        "status": "needs_review",
        "data": { "outcomes": [{
            "item_id": "verify-tdl-001",
            "canonical_task_ids": [TASK],
            "status": "needs_review",
            "summary": "provider rows do not address all eight dimensions",
        }]},
    })
}

fn host_recording(triage: Value) -> Arc<RecordingHost> {
    RecordingHost::new(Box::new(move |_method, call_id| {
        if call_id.starts_with("verification-failure-triage") {
            return triage.clone();
        }
        json!({ "status": "accepted", "summary": "ok", "data": { "items": [] } })
    }))
}

/// Round one: retry is a fair hypothesis, so no writer yet.
#[tokio::test]
async fn the_first_round_retries_without_calling_a_writer() {
    let host = host_recording(triage_with_only_retry_items());
    let driver = driver(host.clone());
    let items = vec![plan_item()];
    let mut verification = failed_verification();
    let mut evidence = LifecycleEvidence::default();
    let mut remediation_attempt = 0usize;

    driver
        .run_verification_remediation(
            &items,
            &items,
            1,
            1,
            1, // repair_attempt: the first round
            &mut remediation_attempt,
            &mut verification,
            &mut evidence,
        )
        .await
        .expect("triage");

    assert!(
        host.ids_starting_with("verification-remediation")
            .is_empty(),
        "round one must not skip straight to a writer: {:?}",
        host.call_ids()
    );
}

/// Round two: the retry did not settle it, so the artifact must change and the
/// writer is asked instead of the verifier being read a third time.
#[tokio::test]
async fn a_persisting_finding_escalates_to_write_remediation() {
    let host = host_recording(triage_with_only_retry_items());
    let driver = driver(host.clone());
    let items = vec![plan_item()];
    let mut verification = failed_verification();
    let mut evidence = LifecycleEvidence::default();
    let mut remediation_attempt = 0usize;

    driver
        .run_verification_remediation(
            &items,
            &items,
            1,
            1,
            2, // repair_attempt: a round already retried and the finding stands
            &mut remediation_attempt,
            &mut verification,
            &mut evidence,
        )
        .await
        .expect("triage");

    assert!(
        !host
            .ids_starting_with("verification-remediation")
            .is_empty(),
        "a persisting content finding must reach a writer: {:?}",
        host.call_ids()
    );
}
