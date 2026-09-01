use std::path::PathBuf;

use archon_workflow::RemediationScope;

use super::workflow_gate::{GateEvaluation, GateFinding, GateId};
use super::workflow_gate_envelope::{StagedGateOutput, stage_gate_evaluation};

#[test]
fn staged_gate_result_writes_exact_envelope_outputs_and_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let staging = temp.path().join("staging");
    let envelope_path = staging.join("gate-envelope.json");
    let live = temp.path().join("live/artifact.json");
    std::fs::create_dir_all(live.parent().unwrap()).unwrap();
    std::fs::write(&live, b"live-before").unwrap();
    let evaluation = GateEvaluation::new(
        "human report",
        vec![GateFinding::new(
            GateId::FreezeSkeleton,
            "candidate needs a narrower edge",
            "TASK-X-010",
            Some(PathBuf::from("candidate.json")),
            RemediationScope::Skeleton,
        )],
    );

    let manifest = stage_gate_evaluation(
        temp.path(),
        &staging,
        &envelope_path,
        "call-1",
        "freeze-skeleton",
        evaluation,
        vec![StagedGateOutput {
            relative_path: "task-skeleton.json".into(),
            bytes: b"candidate-bytes".to_vec(),
        }],
    )
    .unwrap();

    assert_eq!(std::fs::read(&live).unwrap(), b"live-before");
    assert_eq!(manifest.call_id, "call-1");
    assert_eq!(manifest.command_id, "freeze-skeleton");
    assert_eq!(manifest.entries.len(), 2);
    assert_eq!(
        manifest
            .entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect::<Vec<_>>(),
        ["gate-envelope.json", "task-skeleton.json"]
    );
    for entry in &manifest.entries {
        let bytes = std::fs::read(staging.join(&entry.relative_path)).unwrap();
        assert_eq!(entry.byte_len, bytes.len() as u64);
        assert_eq!(
            entry.blake3,
            archon_workflow::task_set_contract::content_digest(&bytes)
        );
    }
    let envelope: archon_workflow::GateEnvelopeV1 =
        serde_json::from_slice(&std::fs::read(&envelope_path).unwrap()).unwrap();
    assert_eq!(envelope.report, "human report");
    assert_eq!(envelope.policy_findings.len(), 1);
    assert_eq!(
        envelope.policy_findings[0].remediation_scope,
        RemediationScope::Skeleton
    );
}

#[test]
fn staged_gate_output_rejects_escape_before_any_write() {
    let temp = tempfile::tempdir().unwrap();
    let staging = temp.path().join("staging");
    let envelope_path = staging.join("gate-envelope.json");
    let outside = temp.path().join("outside.txt");

    let error = stage_gate_evaluation(
        temp.path(),
        &staging,
        &envelope_path,
        "call-1",
        "land-task-body",
        GateEvaluation::new("", Vec::new()),
        vec![StagedGateOutput {
            relative_path: "../outside.txt".into(),
            bytes: b"candidate".to_vec(),
        }],
    )
    .unwrap_err();

    assert!(error.to_string().contains("invalid staged relative path"));
    assert!(!outside.exists());
    assert!(!envelope_path.exists());
}

/// Every staged gate — not just the freezes — must leave its findings readable.
///
/// Staged gates never reach `run_sync_gate`, so this funnel is the only place
/// their finding text is persisted. Patching the freeze wrapper alone left the
/// lint and requirements-trace gates still writing nothing, which is why a live
/// decomposition's body-phase findings stayed unreadable after the first fix.
#[test]
fn every_staged_gate_records_its_finding_text() {
    let temp = tempfile::tempdir().unwrap();
    let staging = temp.path().join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let envelope_path = staging.join("envelope.json");

    let evaluation = GateEvaluation::new(
        "staged lint",
        vec![GateFinding::new(
            crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
            "body declares a frozen field that the skeleton does not carry",
            "TASK-X-010",
            None,
            RemediationScope::Body,
        )],
    );

    stage_gate_evaluation(
        temp.path(),
        &staging,
        &envelope_path,
        "call-9",
        "lint",
        evaluation,
        Vec::new(),
    )
    .expect("staged lint evaluation");

    let text = std::fs::read_to_string(&envelope_path).unwrap_or_else(|error| {
        panic!("staged lint must write {}: {error}", envelope_path.display())
    });
    assert!(
        text.contains("frozen field that the skeleton does not carry"),
        "the envelope must carry the finding text: {text}"
    );
    assert!(
        text.contains("TASK-X-010"),
        "the envelope must name the subject the finding is about: {text}"
    );
    // The staged child prepares; it never commits live state.
    let log = crate::command::workflow_gate::shadow_log_path(temp.path());
    assert!(
        !log.exists(),
        "the staged child must not append to the live shadow log at {}",
        log.display()
    );
}
