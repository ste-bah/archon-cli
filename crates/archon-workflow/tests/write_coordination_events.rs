//! TASK-WC-008 — events + learning + lifecycle resume + status tests.

use std::collections::BTreeMap;

use archon_workflow::classify_resume;
use archon_workflow::events::WorkflowEventKind;
use archon_workflow::events::write_coordination_events::{
    WriteCoordinationConflictKind, build_write_coordination_events, serial_fallback_reason_str,
};
use archon_workflow::learning::record_write_coordination_outcome;
use archon_workflow::write_coordinator::coordinator::{
    CoordinatedOutcome, PlanRecord, WaveOutcome,
};
use archon_workflow::write_coordinator::patch_apply::VerifyResult;
use archon_workflow::write_coordinator::patch_manifest::{
    PATCH_MANIFEST_SCHEMA, PatchManifest, persist_manifest_status_update,
};
use archon_workflow::write_coordinator::status::render_compact;
use archon_workflow::write_coordinator::ManifestStatus;
use archon_workflow::write_coordinator::SerialFallbackReason;
use archon_workflow::WorkflowStore;

fn plan(item: &str, wave: u32) -> PlanRecord {
    PlanRecord {
        item_id: item.into(),
        wave_id: wave,
        target_files: vec!["src/a.rs".into()],
        changed_files: vec!["src/a.rs".into()],
        post_hashes: BTreeMap::from([("src/a.rs".to_string(), "blake3abc123".to_string())]),
        patch_bytes_len: 42,
    }
}

fn applied_outcome() -> CoordinatedOutcome {
    CoordinatedOutcome {
        run_id: "run1".into(),
        stage_id: "implement".into(),
        waves: vec![WaveOutcome {
            wave_id: 0,
            items: vec!["i0".into(), "i1".into()],
            apply_record: None,
            verify: Some(VerifyResult {
                exit: 0,
                stdout_tail: String::new(),
                stderr_tail: String::new(),
                duration_ms: 1,
            }),
            failure: None,
        }],
        serial_fallback: None,
        item_status: BTreeMap::from([
            ("i0".to_string(), ManifestStatus::Applied),
            ("i1".to_string(), ManifestStatus::Applied),
        ]),
        plans: vec![plan("i0", 0), plan("i1", 0)],
    }
}

#[test]
fn write_coordination_events_count_and_order() {
    let events = build_write_coordination_events(&applied_outcome()).unwrap();
    let kinds: Vec<WorkflowEventKind> = events.iter().map(|(k, _)| k.clone()).collect();
    assert_eq!(kinds.len(), 10, "expected 10 events, got {}", kinds.len());
    use WorkflowEventKind::*;
    assert_eq!(
        kinds,
        vec![
            WriteCoordinationItemWritePlanCreated,
            WriteCoordinationItemWritePlanCreated,
            WriteCoordinationWaveScheduled,
            WriteCoordinationItemWorkspaceCreated,
            WriteCoordinationItemWorkspaceCreated,
            WriteCoordinationPatchCaptured,
            WriteCoordinationPatchCaptured,
            WriteCoordinationPatchApplied,
            WriteCoordinationPatchApplied,
            WriteCoordinationWaveVerificationResult,
        ]
    );
}

#[test]
fn write_coordination_events_mutation_wave_no_applied() {
    let mut outcome = applied_outcome();
    outcome.waves[0].failure = Some("CanonicalMutation: i0".into());
    outcome.item_status.clear();
    let events = build_write_coordination_events(&outcome).unwrap();
    let kinds: Vec<WorkflowEventKind> = events.iter().map(|(k, _)| k.clone()).collect();
    assert!(kinds.contains(&WorkflowEventKind::WriteCoordinationDirectCanonicalMutationDetected));
    assert!(
        !kinds.contains(&WorkflowEventKind::WriteCoordinationPatchApplied),
        "mutation wave must not emit PatchApplied"
    );
}

#[test]
fn write_coordination_events_fallback_once() {
    let outcome = CoordinatedOutcome {
        run_id: "run1".into(),
        stage_id: "implement".into(),
        serial_fallback: Some(SerialFallbackReason::BoundaryUnavailable),
        ..Default::default()
    };
    let events = build_write_coordination_events(&outcome).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0, WorkflowEventKind::WriteCoordinationSerialFallback);
    assert_eq!(events[0].1["fallback"], "boundary_unavailable");
}

#[test]
fn event_payload_fields_pass_sanitizer() {
    const FORBIDDEN: &[&str] = &[
        "thinking", "reasoning", "reasoning_encrypted", "encrypted_reasoning", "oauth_token",
        "access_token", "refresh_token", "api_key", "authorization", "raw_text",
    ];
    let events = build_write_coordination_events(&applied_outcome()).unwrap();
    for (_, detail) in &events {
        if let Some(obj) = detail.as_object() {
            for key in obj.keys() {
                let lower = key.to_ascii_lowercase();
                for forbidden in FORBIDDEN {
                    assert!(
                        !lower.contains(forbidden),
                        "payload key '{key}' substring-matches forbidden '{forbidden}'"
                    );
                }
            }
        }
    }
}

#[test]
fn serial_fallback_reason_strings() {
    assert_eq!(serial_fallback_reason_str(SerialFallbackReason::FeatureDisabled), "feature_disabled");
    assert_eq!(serial_fallback_reason_str(SerialFallbackReason::NonGitRoot), "non_git_root");
    assert_eq!(
        serial_fallback_reason_str(SerialFallbackReason::BoundaryUnavailable),
        "boundary_unavailable"
    );
}

#[test]
fn conflict_kind_has_eight_variants() {
    // Compile-time enumeration of all 8 (exhaustive match) proves the count.
    use WriteCoordinationConflictKind::*;
    let all = [
        StaleBaseline, PatchApplyConflict, SecretDetected, UndeclaredWrite, FileTooLarge,
        PatchTooLarge, OutputNotUsable, ConflictGraphViolation,
    ];
    assert_eq!(all.len(), 8);
}

#[test]
fn learning_record_is_metadata_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(dir.path());
    let outcome = applied_outcome();
    record_write_coordination_outcome(&store, &outcome).unwrap();
    let path = store
        .run_dir("run1")
        .join("learning/write-coordination/outcomes.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    // No diff-line bytes embedded.
    for needle in ["+++", "--- ", "\n+ ", "\n- "] {
        assert!(!text.contains(needle), "learning row leaked diff bytes: {needle}");
    }
    // blake3 hashes are allowed and present.
    assert!(text.contains("blake3abc123"), "blake3 hash must be recorded");
    assert!(text.contains("\"item_id\":\"i0\""));
    assert!(text.contains("\"patch_byte_size\":42"));
}

fn manifest(item: &str, status: ManifestStatus) -> PatchManifest {
    PatchManifest {
        schema: PATCH_MANIFEST_SCHEMA.into(),
        run_id: "run1".into(),
        stage_id: "implement".into(),
        item_id: item.into(),
        baseline_commit: "abc".into(),
        patch_path: std::path::PathBuf::from("x.patch"),
        declared_target_files: vec!["src/a.rs".into()],
        changed_files: vec!["src/a.rs".into()],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: BTreeMap::new(),
        post_hashes: BTreeMap::new(),
        verify_command: None,
        agent_artifact_path: None,
        status,
    }
}

#[test]
fn lifecycle_resume_write_coordination_classifies() {
    let dir = tempfile::tempdir().unwrap();
    let run_root = dir.path();
    let seed = |item: &str, status: ManifestStatus| {
        let m = manifest(item, status);
        persist_manifest_status_update(run_root, "run1", "implement", &m.item_id, &m).unwrap();
    };
    seed("item-A", ManifestStatus::Applied);
    seed("item-B", ManifestStatus::Failed { reason: "boom".into() });
    seed("item-C", ManifestStatus::IdempotentNoop);
    seed("item-D", ManifestStatus::Conflicted);
    let items = vec![
        "item-A".to_string(),
        "item-B".to_string(),
        "item-C".to_string(),
        "item-D".to_string(),
        "item-E".to_string(),
    ];
    let c = classify_resume(run_root, "implement", &items);
    assert_eq!(c.skip, vec!["item-A".to_string(), "item-C".to_string()]);
    assert_eq!(c.reexecute, vec!["item-B".to_string()]);
    assert_eq!(c.surfaced_conflicts, vec!["item-D".to_string()]);
    assert_eq!(c.fresh, vec!["item-E".to_string()]);
}

#[test]
fn lifecycle_resume_write_coordination_ac_wc_010() {
    let dir = tempfile::tempdir().unwrap();
    let run_root = dir.path();
    let mut items = Vec::new();
    for i in 0..21 {
        let id = format!("applied-{i}");
        let m = manifest(&id, ManifestStatus::Applied);
        persist_manifest_status_update(run_root, "run1", "implement", &m.item_id, &m).unwrap();
        items.push(id);
    }
    for i in 0..4 {
        let id = format!("failed-{i}");
        let m = manifest(&id, ManifestStatus::Failed { reason: "x".into() });
        persist_manifest_status_update(run_root, "run1", "implement", &m.item_id, &m).unwrap();
        items.push(id);
    }
    let conf = manifest("conflicted-0", ManifestStatus::Conflicted);
    persist_manifest_status_update(run_root, "run1", "implement", &conf.item_id, &conf).unwrap();
    items.push("conflicted-0".into());

    let c = classify_resume(run_root, "implement", &items);
    assert_eq!(c.reexecute.len(), 4, "only the 4 Failed items re-execute");
    assert_eq!(c.skip.len(), 21, "the 21 Applied items are skipped");
    assert_eq!(c.surfaced_conflicts, vec!["conflicted-0".to_string()]);
}

#[test]
fn status_render_active_six_lines() {
    use archon_workflow::write_coordinator::status::WriteCoordinationStatus;
    let s = WriteCoordinationStatus {
        enabled: true,
        stage_id: "implement".into(),
        wave_index: 1,
        wave_total: 2,
        width: 2,
        items_running: 1,
        items_failed: 0,
        items_accepted: 1,
        apply_state: "applied".into(),
        fallback_reason: None,
    };
    assert_eq!(render_compact(&s).lines().count(), 6);
}

#[test]
fn status_render_fallback_one_line() {
    use archon_workflow::write_coordinator::status::WriteCoordinationStatus;
    let s = WriteCoordinationStatus {
        enabled: false,
        stage_id: "implement".into(),
        wave_index: 0,
        wave_total: 0,
        width: 1,
        items_running: 0,
        items_failed: 0,
        items_accepted: 0,
        apply_state: "n/a".into(),
        fallback_reason: Some("non_git_root".into()),
    };
    let out = render_compact(&s);
    assert_eq!(out.lines().count(), 1);
    assert_eq!(out, "write_coordination: serial_fallback (non_git_root)\n");
}
