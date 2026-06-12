//! TASK-WC-008 — typed payloads + emission for write-coordination events (§18).
//!
//! Each event variant on `WorkflowEventKind` is bare; the typed payload travels
//! in `WorkflowEvent.detail`. Field names avoid the FORBIDDEN_FIELDS substrings
//! so `sanitize_value` never redacts a legitimate field.

use serde::Serialize;
use serde_json::Value;

use super::{WorkflowEventKind, WorkflowEventLog};
use crate::error::WorkflowResult;
use crate::write_coordinator::ManifestStatus;
use crate::write_coordinator::SerialFallbackReason;
use crate::write_coordinator::coordinator::CoordinatedOutcome;

/// The kind of write-coordination conflict surfaced in a PatchConflict event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteCoordinationConflictKind {
    StaleBaseline,
    PatchApplyConflict,
    SecretDetected,
    UndeclaredWrite,
    FileTooLarge,
    PatchTooLarge,
    OutputNotUsable,
    ConflictGraphViolation,
}

#[derive(Serialize)]
struct PlanCreatedPayload {
    run_id: String,
    stage_id: String,
    item_id: String,
    target_files: Vec<String>,
}

#[derive(Serialize)]
struct WaveScheduledPayload {
    run_id: String,
    stage_id: String,
    wave_id: u32,
    items: Vec<String>,
    width: usize,
}

#[derive(Serialize)]
struct WorkspacePayload {
    run_id: String,
    stage_id: String,
    item_id: String,
    wave_id: u32,
}

#[derive(Serialize)]
struct PatchCapturedPayload {
    run_id: String,
    stage_id: String,
    item_id: String,
    changed_files: Vec<String>,
    patch_byte_size: usize,
    blake3_hashes: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize)]
struct PatchAppliedPayload {
    run_id: String,
    stage_id: String,
    item_id: String,
    wave_id: u32,
}

#[derive(Serialize)]
struct PatchConflictPayload {
    run_id: String,
    stage_id: String,
    item_id: String,
    conflict_kind: WriteCoordinationConflictKind,
    detail: String,
}

#[derive(Serialize)]
struct VerifyPayload {
    run_id: String,
    stage_id: String,
    wave_id: u32,
    exit: i32,
}

#[derive(Serialize)]
struct MutationPayload {
    run_id: String,
    stage_id: String,
    wave_id: u32,
    detail: String,
}

#[derive(Serialize)]
struct FallbackPayload {
    run_id: String,
    stage_id: String,
    fallback: String,
}

pub fn serial_fallback_reason_str(reason: SerialFallbackReason) -> &'static str {
    match reason {
        SerialFallbackReason::FeatureDisabled => "feature_disabled",
        SerialFallbackReason::NonGitRoot => "non_git_root",
        SerialFallbackReason::BoundaryUnavailable => "boundary_unavailable",
    }
}

/// Build the ordered (kind, detail) event list for an outcome. Pure — no I/O,
/// so it is directly assertable in tests.
pub fn build_write_coordination_events(
    outcome: &CoordinatedOutcome,
) -> WorkflowResult<Vec<(WorkflowEventKind, Value)>> {
    let mut events = Vec::new();
    if let Some(reason) = outcome.serial_fallback {
        events.push((
            WorkflowEventKind::WriteCoordinationSerialFallback,
            serde_json::to_value(FallbackPayload {
                run_id: outcome.run_id.clone(),
                stage_id: outcome.stage_id.clone(),
                fallback: serial_fallback_reason_str(reason).into(),
            })?,
        ));
        return Ok(events);
    }
    for plan in &outcome.plans {
        events.push((
            WorkflowEventKind::WriteCoordinationItemWritePlanCreated,
            serde_json::to_value(PlanCreatedPayload {
                run_id: outcome.run_id.clone(),
                stage_id: outcome.stage_id.clone(),
                item_id: plan.item_id.clone(),
                target_files: plan.target_files.clone(),
            })?,
        ));
    }
    for wave in &outcome.waves {
        push_wave_events(outcome, wave, &mut events)?;
    }
    Ok(events)
}

fn push_wave_events(
    outcome: &CoordinatedOutcome,
    wave: &crate::write_coordinator::WaveOutcome,
    events: &mut Vec<(WorkflowEventKind, Value)>,
) -> WorkflowResult<()> {
    events.push((
        WorkflowEventKind::WriteCoordinationWaveScheduled,
        serde_json::to_value(WaveScheduledPayload {
            run_id: outcome.run_id.clone(),
            stage_id: outcome.stage_id.clone(),
            wave_id: wave.wave_id,
            items: wave.items.clone(),
            width: wave.items.len(),
        })?,
    ));
    for item in &wave.items {
        events.push((
            WorkflowEventKind::WriteCoordinationItemWorkspaceCreated,
            serde_json::to_value(WorkspacePayload {
                run_id: outcome.run_id.clone(),
                stage_id: outcome.stage_id.clone(),
                item_id: item.clone(),
                wave_id: wave.wave_id,
            })?,
        ));
    }
    if let Some(failure) = &wave.failure {
        events.push((
            WorkflowEventKind::WriteCoordinationDirectCanonicalMutationDetected,
            serde_json::to_value(MutationPayload {
                run_id: outcome.run_id.clone(),
                stage_id: outcome.stage_id.clone(),
                wave_id: wave.wave_id,
                detail: failure.clone(),
            })?,
        ));
        return Ok(());
    }
    push_item_outcome_events(outcome, wave, events)?;
    Ok(())
}

fn push_item_outcome_events(
    outcome: &CoordinatedOutcome,
    wave: &crate::write_coordinator::WaveOutcome,
    events: &mut Vec<(WorkflowEventKind, Value)>,
) -> WorkflowResult<()> {
    for plan in outcome.plans.iter().filter(|p| p.wave_id == wave.wave_id) {
        events.push((
            WorkflowEventKind::WriteCoordinationPatchCaptured,
            serde_json::to_value(PatchCapturedPayload {
                run_id: outcome.run_id.clone(),
                stage_id: outcome.stage_id.clone(),
                item_id: plan.item_id.clone(),
                changed_files: plan.changed_files.clone(),
                patch_byte_size: plan.patch_bytes_len,
                blake3_hashes: plan.post_hashes.clone(),
            })?,
        ));
    }
    for item in &wave.items {
        match outcome.item_status.get(item) {
            Some(ManifestStatus::Applied) => events.push((
                WorkflowEventKind::WriteCoordinationPatchApplied,
                serde_json::to_value(PatchAppliedPayload {
                    run_id: outcome.run_id.clone(),
                    stage_id: outcome.stage_id.clone(),
                    item_id: item.clone(),
                    wave_id: wave.wave_id,
                })?,
            )),
            Some(ManifestStatus::Failed { reason }) => events.push((
                WorkflowEventKind::WriteCoordinationPatchConflict,
                serde_json::to_value(PatchConflictPayload {
                    run_id: outcome.run_id.clone(),
                    stage_id: outcome.stage_id.clone(),
                    item_id: item.clone(),
                    conflict_kind: classify_conflict(reason),
                    detail: reason.clone(),
                })?,
            )),
            _ => {}
        }
    }
    if let Some(verify) = &wave.verify {
        events.push((
            WorkflowEventKind::WriteCoordinationWaveVerificationResult,
            serde_json::to_value(VerifyPayload {
                run_id: outcome.run_id.clone(),
                stage_id: outcome.stage_id.clone(),
                wave_id: wave.wave_id,
                exit: verify.exit,
            })?,
        ));
    }
    Ok(())
}

fn classify_conflict(reason: &str) -> WriteCoordinationConflictKind {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("stale") {
        WriteCoordinationConflictKind::StaleBaseline
    } else if lower.contains("secret") {
        WriteCoordinationConflictKind::SecretDetected
    } else if lower.contains("undeclared") {
        WriteCoordinationConflictKind::UndeclaredWrite
    } else {
        WriteCoordinationConflictKind::PatchApplyConflict
    }
}

/// Best-effort observability: emit §18 events + write metadata-only learning
/// rows. Logging failures never propagate (the stage outcome is unaffected).
pub fn emit_and_record(
    store: &crate::store::WorkflowStore,
    seq_base: u64,
    outcome: &CoordinatedOutcome,
) {
    let log = WorkflowEventLog::new(store.clone());
    let _ = emit_write_coordination_events(&log, outcome, seq_base);
    let _ = crate::learning::record_write_coordination_outcome(store, outcome);
}

/// Emit every write-coordination event for an outcome via the existing event
/// log API, starting at `seq_start`. Returns the next free seq.
pub fn emit_write_coordination_events(
    log: &WorkflowEventLog,
    outcome: &CoordinatedOutcome,
    seq_start: u64,
) -> WorkflowResult<u64> {
    let mut seq = seq_start;
    for (kind, detail) in build_write_coordination_events(outcome)? {
        log.emit(&outcome.run_id, seq, kind, detail)?;
        seq += 1;
    }
    Ok(seq)
}
