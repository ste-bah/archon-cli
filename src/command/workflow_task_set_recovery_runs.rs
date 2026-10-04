//! Run discovery and proofs for a recovery shared by several launch anchors.
use super::*;

pub(super) fn store(pin: &Path) -> Result<WorkflowStore> {
    let root = pin
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("pin has no store"))?;
    Ok(WorkflowStore::new(root.join("workflows")))
}

/// Use the surviving, archived contract rather than directory ordering to
/// select a no-prior-pin template. Older receipts can use their captured
/// skeleton or a surviving launch preimage instead. Select only before
/// completion; verification must use the source the completion recorded.
pub(super) fn anchor(
    record: &Recovery,
    pin: &Path,
) -> Result<Option<PortableAcceptanceIdentityV1>> {
    if let Some(prior) = &record.prior {
        return Ok(Some(prior.identity()));
    }
    let digest = record.contract_digest.as_ref().or_else(|| {
        record
            .skeleton
            .as_ref()
            .map(|skeleton| &skeleton.acceptance_digest)
    });
    let history = ChainHistory::for_pin(pin);
    for launch in record.runs.values() {
        if digest.is_some_and(|digest| digest != &launch.acceptance_digest) {
            continue;
        }
        if history.get(&launch.acceptance_digest)?.is_none() {
            continue;
        }
        if let Some(digest) = &launch.skeleton_digest {
            let Some(bytes) = history.get(digest)? else {
                continue;
            };
            if let Some(skeleton) = &record.skeleton
                && serde_json::from_slice::<TaskSkeleton>(&bytes)? != *skeleton
            {
                continue;
            }
        } else if record.skeleton.is_some() {
            continue;
        }
        return Ok(Some(launch.clone()));
    }
    Ok(None)
}

/// Bind the completed hop to the captured authority without reselecting
/// its source from history that later preimage imports can extend.
pub(super) fn completed_anchor(record: &Recovery) -> Result<&PortableAcceptanceIdentityV1> {
    let from = &record
        .completed
        .as_ref()
        .and_then(|done| done.lineage.last())
        .ok_or_else(|| anyhow!("recovery completion has no prior anchor"))?
        .from;
    let captured = match &record.prior {
        Some(prior) => prior.identity() == *from,
        None => {
            record.runs.values().any(|launch| launch == from)
                && record
                    .contract_digest
                    .as_ref()
                    .is_none_or(|digest| digest == &from.acceptance_digest)
        }
    };
    if !captured {
        return Err(anyhow!(
            "recovery completion does not bind its prior anchor"
        ));
    }
    Ok(from)
}

/// An unreadable snapshot does not permanently disqualify a discovered run.
/// Its id must have been logged before unfreeze, and its restored snapshot
/// must bind the same root and identity supplied by the reader.
pub(super) fn authorized(
    record: &Recovery,
    pin: &Path,
    run: &str,
    launch: &PortableAcceptanceIdentityV1,
    launch_lineage: LaunchLineage,
) -> Result<bool> {
    if let Some(captured) = record.runs.get(run) {
        return Ok(captured == launch);
    }
    if !record.skipped_runs.contains(run) {
        return Ok(false);
    }
    let store = store(pin)?;
    let metadata = store.run_dir(run).join("v2/generated-metadata.json");
    let finalization = store.run_dir(run).join("v2/finalization.json");
    validate_existing_parents(&metadata, store.root())?;
    validate_existing_parents(&finalization, store.root())?;
    let snapshot = crate::command::acceptance_chain::launch_snapshot(&store, run)?;
    Ok(
        snapshot.portable_acceptance_identity.as_ref() == Some(launch)
            && LaunchLineage::from_marker(snapshot.lineage_recording) == launch_lineage
            && Path::new(&snapshot.canonical_task_root_identity)
                .canonicalize()
                .map(archon_shell::paths::plain)
                .ok()
                .as_ref()
                == Some(&record.task_root),
    )
}

/// Prove adoption from this run's launch, never from another run's contract.
/// The caller authenticates the shared completion against durable authority.
/// When the lineage does not start at this launch, derive its recovery hop
/// from that authority, retaining ordinary contract and skeleton checks.
pub(super) fn proof(
    record: &Recovery,
    launch: &PortableAcceptanceIdentityV1,
    launch_lineage: LaunchLineage,
    pin: &AcceptancePin,
    pin_path: &Path,
    tasks: &Path,
) -> Result<ChainProof> {
    let history = ChainHistory::for_pin(pin_path);
    if pin.lineage.iter().any(|link| link.from == *launch) {
        return Ok(verify_reached_from(
            launch,
            launch_lineage,
            pin,
            tasks,
            &history,
        )?);
    }
    let done = record
        .completed
        .as_ref()
        .ok_or_else(|| anyhow!("recovery has no completion"))?;
    // Validate the original later links before rehashing them for this run.
    // Rebuilding the proof must not repair a corrupt digest or a chain gap.
    for links in pin.lineage[done.lineage.len() - 1..].windows(2) {
        let (before, link) = (&links[0], &links[1]);
        if link.prior_link_digest.as_deref() != Some(before.digest().as_str())
            || link.from != before.to
        {
            return Err(anyhow!(
                "chain check lineage_broken failed: recovery followup does not bind its preceding link"
            ));
        }
    }
    let mut view = pin.clone();
    view.lineage = vec![PinTransition::extending(
        &[],
        launch.clone(),
        done.identity.clone(),
        done.ids.clone(),
        &format!("{TRIGGER}{}", record.transaction),
    )];
    for link in &pin.lineage[done.lineage.len()..] {
        view.lineage.push(PinTransition::extending(
            &view.lineage,
            link.from.clone(),
            link.to.clone(),
            link.reauthored_ids.clone(),
            &link.trigger,
        ));
    }
    Ok(verify_reached_from(
        launch,
        launch_lineage,
        &view,
        tasks,
        &history,
    )?)
}
