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
