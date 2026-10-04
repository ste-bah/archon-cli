//! Acceptance chain publication, including a recovery re-freeze receipt.
use super::*;

#[cfg(test)]
pub(crate) fn publish_acceptance_files(
    tasks_root: &Path,
    project_root: &Path,
    contract_bytes: &[u8],
    lock: &AcceptanceLock,
    pin: &AcceptancePin,
) -> Result<()> {
    publish_acceptance_files_with_recovery(
        tasks_root,
        project_root,
        contract_bytes,
        lock,
        pin,
        None,
    )
}

pub(crate) fn publish_acceptance_files_with_recovery(
    tasks_root: &Path,
    project_root: &Path,
    contract_bytes: &[u8],
    lock: &AcceptanceLock,
    pin: &AcceptancePin,
    recovery: Option<&super::super::recovery_lineage::Publication>,
) -> Result<()> {
    let pin_path = super::super::acceptance_pin_path(project_root, tasks_root);
    if let Some(parent) = pin_path.parent() {
        create_dir_all_durably(parent)?;
    }
    let _lock = ChainLock::acquire(&pin_path, tasks_root)?;
    // PLAN-11: the sources each check runs are pinned with the chain.
    let sidecar = super::super::check_sources::frozen_sidecar(
        project_root,
        tasks_root,
        contract_bytes,
        None,
    )?;
    let mut pin = pin.clone();
    pin.check_sources_digest = Some(content_digest(&sidecar.1));
    let mut files = vec![
        (
            tasks_root.join(ACCEPTANCE_CONTRACT_FILE),
            contract_bytes.to_vec(),
        ),
        (
            tasks_root.join(ACCEPTANCE_LOCK_FILE),
            serde_json::to_vec_pretty(lock)?,
        ),
        sidecar,
        (pin_path.clone(), serde_json::to_vec_pretty(&pin)?),
    ];
    let mut expected = Vec::new();
    if let Some(recovery) = recovery {
        files.extend(recovery.files.clone());
        expected.push(recovery.expected.clone());
    }
    let transaction = begin_publish(
        &pin_path,
        tasks_root,
        &files,
        "workflow freeze-acceptance",
        &expected,
    )?;
    for warning in transaction.commit()? {
        eprintln!("warning: {warning}");
    }
    Ok(())
}
