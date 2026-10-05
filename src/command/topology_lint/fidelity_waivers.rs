//! Obligation fidelity waivers: `--waive-obligation` flags become records,
//! records live verbatim in the task set's freeze pin, and both gates read
//! them back from there. Split from `fidelity.rs` so the audit file stays
//! under its size budget; the audit itself is unchanged.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::fidelity_audit::ObligationWaiver;
use archon_workflow::task_set_contract::{AcceptancePin, content_digest};

/// `--waive-obligation <ID>… --waive-reason <TEXT>` as recorded waivers.
pub(crate) fn waivers_from_flags(
    ids: &[String],
    reason: Option<&str>,
) -> Result<Vec<ObligationWaiver>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let reason = reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .ok_or_else(|| anyhow!("--waive-obligation requires --waive-reason \"<text>\"; a waiver without a reason is not auditable"))?;
    let waived_at = chrono::Utc::now().to_rfc3339();
    Ok(ids
        .iter()
        .map(|id| ObligationWaiver {
            obligation_id: id.trim().to_string(),
            reason: reason.to_string(),
            waived_at: waived_at.clone(),
            binary_commit: env!("ARCHON_GIT_HASH").into(),
        })
        .collect())
}

/// Record waivers verbatim in the task set's freeze pin, replacing an earlier
/// waiver for the same id. A set that was never frozen has no pin to carry
/// the record, so the waiver is refused rather than kept somewhere unaudited.
///
/// Issue 336: the pin is read and rewritten under the chain lock every
/// freeze, republish and repin of the set holds, once a publish a crash
/// interrupted is settled, and the rewrite is one journaled publish that
/// refuses a pin changed since it was read: a waiver never overwrites a
/// concurrent publish, nor is it lost under one.
pub(crate) fn record_waivers(
    cwd: &Path,
    tasks_root: &Path,
    waivers: &[ObligationWaiver],
) -> Result<PathBuf> {
    use crate::command::workflow_task_set::{ChainLock, acceptance_pin_path, begin_publish};
    let pin_path = acceptance_pin_path(cwd, tasks_root);
    let missing = || {
        format!(
            "no acceptance freeze pin at {} for {}; a waiver attaches to a frozen task set, so freeze first",
            pin_path.display(),
            tasks_root.display()
        )
    };
    if !pin_path.is_file() {
        return Err(anyhow!(missing()));
    }
    let _chain = ChainLock::acquire(&pin_path, tasks_root)?;
    let bytes = std::fs::read(&pin_path).with_context(missing)?;
    let mut pin: AcceptancePin = serde_json::from_slice(&bytes)
        .with_context(|| format!("acceptance pin {} is malformed", pin_path.display()))?;
    for waiver in waivers {
        pin.fidelity_waivers
            .retain(|existing| existing.obligation_id != waiver.obligation_id);
        pin.fidelity_waivers.push(waiver.clone());
    }
    let prior = [(pin_path.clone(), Some(content_digest(&bytes)))];
    let transaction = begin_publish(
        &pin_path,
        tasks_root,
        &[(pin_path.clone(), serde_json::to_vec_pretty(&pin)?)],
        "workflow lint --waive-obligation",
        &prior,
    )
    .with_context(|| format!("publishing {}", pin_path.display()))?;
    for warning in transaction.commit()? {
        tracing::warn!("{warning}");
        eprintln!("warning: {warning}");
    }
    Ok(pin_path)
}

/// The waivers a frozen task set already carries; none when it has no pin.
pub(crate) fn recorded_waivers(cwd: &Path, tasks_root: &Path) -> Vec<ObligationWaiver> {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(cwd, tasks_root);
    std::fs::read(&pin_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AcceptancePin>(&bytes).ok())
        .map(|pin| pin.fidelity_waivers)
        .unwrap_or_default()
}
