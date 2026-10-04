//! REM-13: a fresh in-run freeze is published exactly as
//! `workflow freeze-acceptance` publishes, under the enforce gate; split
//! from `acceptance_author` for size.

use std::path::Path;

use archon_workflow::task_set_contract::{ACCEPTANCE_CONTRACT_FILE, AcceptanceContract};

use super::super::exec::StageContext;

/// Why a publication was refused: every gate finding (subject, text), and
/// the refusal.
pub(super) struct Refused {
    pub(super) findings: Vec<(String, String)>,
    pub(super) error: String,
}

/// Freeze `contract` exactly as `workflow freeze-acceptance` publishes, under
/// the enforce gate: every finding refuses it.
pub(super) fn publish_fresh(
    context: &StageContext,
    prd_path: &Path,
    contract: &AcceptanceContract,
) -> Result<String, Refused> {
    use crate::command::workflow_gate::{GateEvaluation, GateId, run_sync_gate};
    let refused = |error: anyhow::Error| Refused {
        findings: Vec::new(),
        error: format!("{error:#}"),
    };
    let mode = archon_core::config::GateMode::Enforce;
    let mut prepared = crate::command::workflow_task_set::prepare_from_judged(
        &context.project,
        &context.task_root,
        prd_path,
        mode,
        contract,
    )
    .map_err(refused)?;
    prepared.record_recovery_refreeze().map_err(refused)?;
    let findings: Vec<(String, String)> = (prepared.findings.iter())
        .map(|finding| (finding.subject.clone(), finding.text.clone()))
        .collect();
    let identity = prepared.publication_identity();
    let gate_findings = prepared.findings.clone();
    let mut disposition = run_sync_gate(&context.project, mode, GateId::FreezeAcceptance, || {
        Ok(GateEvaluation::new("", gate_findings).with_publication_identity(identity))
    })
    .map_err(refused)?;
    if let Err(error) = disposition.require_allowed() {
        return Err(Refused {
            findings,
            error: format!("{error:#}"),
        });
    }
    let permit = disposition
        .take_publication_permit()
        .ok_or_else(|| Refused {
            findings: Vec::new(),
            error: "the acceptance freeze received no publication permit".into(),
        })?;
    let result = crate::command::workflow_task_set::publish_acceptance_freeze(prepared, permit)
        .map_err(refused)?;
    debug_assert!(context.task_root.join(ACCEPTANCE_CONTRACT_FILE).exists());
    Ok(result.freeze_event_id)
}
