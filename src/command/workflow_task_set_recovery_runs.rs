//! Run discovery and proofs for a recovery shared by several launch anchors.
use super::*;

pub(super) fn store(pin: &Path) -> Result<WorkflowStore> {
    let root = pin
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("pin has no store"))?;
    Ok(WorkflowStore::new(root.join("workflows")))
}

/// Select an authenticated contract/skeleton pair matching the captured live
/// shape. Prefer the surviving contract among compatible pairs. Select only before
/// completion. Verification authenticates the recorded source with
/// `completed_anchor`, rather than reselecting from surviving preimages.
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
    let mut launches = record.runs.values().collect::<Vec<_>>();
    launches.sort_by_key(|launch| digest.is_some_and(|digest| digest != &launch.acceptance_digest));
    for launch in launches {
        if history.get(&launch.acceptance_digest)?.is_none() {
            continue;
        }
        if let Some(digest) = &launch.skeleton_digest {
            let Some(bytes) = history.get(digest)? else {
                continue;
            };
            if let Some(skeleton) = &record.skeleton {
                let mut candidate: TaskSkeleton = serde_json::from_slice(&bytes)?;
                // Recovery re-binds only the contract digest; task fields must
                // match the authenticated skeleton captured by recovery.
                candidate.acceptance_digest = skeleton.acceptance_digest.clone();
                if candidate != *skeleton {
                    continue;
                }
            }
        } else if record.skeleton.is_some() {
            continue;
        }
        return Ok(Some(launch.clone()));
    }
    if !record.runs.is_empty() {
        return Err(anyhow!(
            "chain check skeleton_changed failed: recovery has no compatible authenticated contract/skeleton pair; restore the launch preimages and retry re-freeze, or re-freeze and start a new run"
        ));
    }
    Ok(None)
}

/// Bind the completed hop to the captured authority without reselecting
/// its source from history that later preimage imports can extend.
pub(super) fn completed_anchor<'a>(
    record: &'a Recovery,
    pin: &Path,
) -> Result<&'a PortableAcceptanceIdentityV1> {
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
                && record.contract_digest.as_ref().is_none_or(|digest| {
                    digest == &from.acceptance_digest
                        || record.skeleton.as_ref().is_some_and(|skeleton| {
                            skeleton.acceptance_digest == from.acceptance_digest
                        })
                })
        }
    };
    if !captured {
        return Err(anyhow!(
            "recovery completion does not bind its prior anchor"
        ));
    }
    // Authenticate this completed source, never reselect it from history.
    // Its contract may be the captured live contract or the captured skeleton's
    // authenticated binding, but its skeleton must actually belong to it.
    if record.prior.is_none()
        && let Some(expected) = &record.skeleton
    {
        let digest = from.skeleton_digest.as_ref().ok_or_else(|| {
            anyhow!("recovery completion does not bind its prior anchor: skeleton missing")
        })?;
        let bytes = ChainHistory::for_pin(pin).get(digest)?.ok_or_else(|| anyhow!(
            "recovery completion does not bind its prior anchor: skeleton preimage {digest} missing; restore its evidence and retry re-freeze"))?;
        let mut skeleton: TaskSkeleton = serde_json::from_slice(&bytes)?;
        if skeleton.acceptance_digest != from.acceptance_digest {
            return Err(anyhow!(
                "recovery completion does not bind its prior anchor: skeleton belongs to another contract"
            ));
        }
        skeleton.acceptance_digest = expected.acceptance_digest.clone();
        if skeleton != *expected {
            return Err(anyhow!(
                "recovery completion does not bind its prior anchor: skeleton shape differs"
            ));
        }
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
