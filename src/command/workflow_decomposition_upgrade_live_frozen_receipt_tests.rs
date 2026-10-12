//! Record shapes of a long-lived observe-mode run: every freeze receipt names
//! the shared acceptance pin, the skeleton freeze rewrites that pin after the
//! acceptance freeze, and a later resume leaves an interrupted, verdict-less
//! acceptance attempt behind.

use super::*;

struct Entry {
    relative: &'static str,
    path: PathBuf,
    prior: Option<String>,
}

fn entry(relative: &'static str, path: PathBuf, prior: Option<String>) -> Entry {
    Entry {
        relative,
        path,
        prior,
    }
}

fn digest_of(path: &Path) -> String {
    archon_workflow::task_set_contract::content_digest(&std::fs::read(path).unwrap())
}

/// A receipt for `entries` as they are on disk now, with the run-local gate
/// envelope every staged freeze also publishes.
fn receipt(
    store: &WorkflowStore,
    run_id: &str,
    capability: &str,
    id: &str,
    entries: Vec<Entry>,
) -> serde_json::Value {
    let envelope = store
        .run_dir(run_id)
        .join("host-command-results")
        .join(id)
        .join("gate-envelope.json");
    std::fs::create_dir_all(envelope.parent().unwrap()).unwrap();
    std::fs::write(&envelope, br#"{"report":"","schema_version":1}"#).unwrap();
    let entries: Vec<_> = entries
        .into_iter()
        .chain([entry("gate-envelope.json", envelope, None)])
        .map(|entry| {
            let bytes = std::fs::read(&entry.path).unwrap();
            let mut value = serde_json::json!({
                "relative_path": entry.relative,
                "destination_path": entry.path,
                "byte_len": bytes.len(),
                "blake3": archon_workflow::task_set_contract::content_digest(&bytes),
            });
            if let Some(prior) = entry.prior {
                value["prior_blake3"] = prior.into();
            }
            value
        })
        .collect();
    serde_json::json!({
        "schema_version": 1, "call_id": id, "command_id": capability,
        "committed_at": "2026-10-10T10:00:00Z", "entries": entries
    })
}

#[allow(clippy::too_many_arguments)]
fn save(
    store: &WorkflowStore,
    run_id: &str,
    capability: &str,
    id: &str,
    sequence: Option<u64>,
    status: &str,
    version: u32,
    findings: serde_json::Value,
    receipt: serde_json::Value,
) {
    let (digest, _) = crate::command::workflow_host_command_logic::digests()
        .remove(capability)
        .unwrap();
    let current = crate::command::workflow_host_command_logic::versions()[capability];
    let interrupted = findings.is_null();
    let data = serde_json::json!({
        "exitCode": if interrupted { serde_json::Value::Null } else { 0.into() },
        "stdout": "", "stderr": "", "stdoutBytes": 0, "stderrBytes": 0,
        "timedOut": false, "interrupted": interrupted,
        "stdoutTruncated": false, "stderrTruncated": false,
        "gateEnvelope": if interrupted { serde_json::Value::Null } else {
            serde_json::json!({"schema_version": 1, "report": "", "policy_findings": findings})
        },
        "publicationReceipt": receipt,
        "postcondition": if interrupted { serde_json::Value::Null } else {
            serde_json::json!({"satisfied": receipt.is_object(), "summary": "fixture"})
        },
        "logicVersion": version,
        "logicDigest": if version == current { digest } else { "earlier-logic-digest".into() },
        "logicBuild": if version == current {
            crate::command::workflow_host_command_logic::THIS_BUILD
        } else {
            "earlier-build"
        }
    });
    let mut record = serde_json::json!({
        "run_id": run_id,
        "call": {"id": id, "method": "hostCommand",
                 "options": {"host_command": {"commandId": capability, "stdin": "{\"entries\":[]}"}}},
        "attempt": 1, "started_at": "2026-10-10T10:00:00Z", "input_hash": "fixture",
        "status": status, "result": {"status": status, "summary": "fixture", "data": data}
    });
    if let Some(sequence) = sequence {
        record["admission_sequence"] = sequence.into();
    }
    archon_workflow::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .save_call_record(&serde_json::from_value(record).unwrap())
        .unwrap();
}

/// Acceptance, then skeleton, each frozen and recorded with its real receipt;
/// `pin_link` overrides the skeleton receipt's prior pin digest.
async fn chained_run(
    versions_behind: u32,
    pin_link: Option<&str>,
) -> (tempfile::TempDir, WorkflowStore, String, String) {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let (task_root, prd) = (
        PathBuf::from(&state.identity.task_root_identity),
        PathBuf::from(&state.identity.prd_identity),
    );
    let support = crate::command::workflow_decompose_frozen_chain::test_support::freeze_acceptance;
    let pin = crate::command::workflow_task_set::acceptance_pin_path(project.path(), &task_root);
    let versions = crate::command::workflow_host_command_logic::versions();
    let behind = |capability: &str| versions[capability] - versions_behind;
    let acceptance_digest = support(project.path(), &prd, &task_root);
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE as CONTRACT, ACCEPTANCE_LOCK_FILE as LOCK,
        TASK_SKELETON_FILE as SKELETON, TASK_SKELETON_LOCK_FILE as SKELETON_LOCK,
    };
    let accepted = receipt(
        &store,
        &run_id,
        "freeze-acceptance",
        "acceptance-judged",
        vec![
            entry(CONTRACT, task_root.join(CONTRACT), None),
            entry(LOCK, task_root.join(LOCK), None),
            entry("acceptance-pin.json", pin.clone(), None),
        ],
    );
    // An earlier observe-mode attempt that published with findings.
    save(
        &store,
        &run_id,
        "freeze-acceptance",
        "acceptance-observed",
        None,
        "needs_review",
        behind("freeze-acceptance"),
        serde_json::json!([{"text": "observe-mode finding", "subject": "acceptance", "remediation_scope": "candidate_artifact"}]),
        accepted.clone(),
    );
    save(
        &store,
        &run_id,
        "freeze-acceptance",
        "acceptance-judged",
        Some(156),
        "accepted",
        behind("freeze-acceptance"),
        serde_json::json!([]),
        accepted,
    );
    let acceptance_pin = digest_of(&pin);
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_skeleton(
        project.path(),
        &task_root,
        &acceptance_digest,
        &["TASK-X-010"],
    );
    let link = pin_link.map_or(acceptance_pin, str::to_string);
    let skeleton = receipt(
        &store,
        &run_id,
        "freeze-skeleton",
        "skeleton-judged",
        vec![
            entry(SKELETON, task_root.join(SKELETON), None),
            entry(SKELETON_LOCK, task_root.join(SKELETON_LOCK), None),
            entry("acceptance-pin.json", pin, Some(link)),
        ],
    );
    save(
        &store,
        &run_id,
        "freeze-skeleton",
        "skeleton-judged",
        Some(157),
        "accepted",
        behind("freeze-skeleton"),
        serde_json::json!([]),
        skeleton,
    );
    // The resume this build replaces, interrupted before any verdict.
    save(
        &store,
        &run_id,
        "freeze-acceptance",
        "acceptance-interrupted",
        Some(243),
        "needs_review",
        versions["freeze-acceptance"],
        serde_json::Value::Null,
        serde_json::Value::Null,
    );
    (project, store, run_id, state.identity.prd_identity)
}

async fn dry_run(project: &tempfile::TempDir, run_id: &str) -> String {
    crate::command::workflow_decompose::dry_run_fixed_decomposition(
        project.path(),
        run_id,
        &launch_config(project.path()),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn pin_rewritten_by_the_skeleton_freeze_still_verifies_both_phases() {
    let (project, _store, run_id, _) = chained_run(1, None).await;
    let output = dry_run(&project, &run_id).await;
    assert!(
        output.contains("frozen_chain_override acceptance=verify reason=judged attempt acceptance-judged accepted with 0 findings"),
        "{output}"
    );
    assert!(
        output.contains("acceptance-pin.json by skeleton-judged"),
        "{output}"
    );
    assert!(
        output.contains(
            "frozen_chain_override skeleton=verify reason=judged attempt skeleton-judged"
        ),
        "{output}"
    );
    let capabilities: Vec<&str> = output
        .lines()
        .filter_map(|line| line.split(" capability=").nth(1))
        .collect();
    assert_eq!(
        capabilities.first().copied(),
        Some("verify-frozen-acceptance"),
        "{output}"
    );
    assert_eq!(
        capabilities.get(1).copied(),
        Some("verify-frozen-skeleton"),
        "{output}"
    );
}

#[tokio::test]
async fn pin_write_no_recorded_publication_explains_keeps_the_chain() {
    let (project, _store, run_id, _) = chained_run(1, Some("unrecorded-pin-digest")).await;
    let output = dry_run(&project, &run_id).await;
    assert!(
        output.contains("frozen_chain_override acceptance=keep reason=judged attempt acceptance-judged receipt does not match disk: acceptance-pin.json was replaced by freeze-skeleton skeleton-judged from unrecorded-pin-digest"),
        "{output}"
    );
    assert!(
        output.contains("frozen_chain_override skeleton=keep reason=acceptance is kept"),
        "{output}"
    );
    assert!(!output.contains("capability=verify-frozen"), "{output}");
}

#[tokio::test]
async fn reusable_freeze_on_the_same_logic_keeps_the_chain() {
    let (project, _store, run_id, _) = chained_run(0, None).await;
    let output = dry_run(&project, &run_id).await;
    assert!(
        output.contains("frozen_chain_override acceptance=keep reason=judged attempt acceptance-judged is reusable by this build"),
        "{output}"
    );
    assert!(!output.contains("capability=verify-frozen"), "{output}");
}
