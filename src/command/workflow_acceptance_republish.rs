//! Re-author named checks and atomically republish the frozen chain.
//! Used by the acceptance repair command and authored-run launch guard.

use archon_workflow::task_set_contract::{
    ACCEPTANCE_LOCK_FILE, FreezeGateMode, TASK_SKELETON_LOCK_FILE,
};
use archon_workflow::task_set_lineage::{ChainHistory, PinTransition};
use archon_workflow::task_skeleton::validate_full_chain;
use std::collections::BTreeMap;

use super::reauthor::{AuthorScope, ReauthorGate};
use super::*;

/// Per-check repair of the contract frozen beside `tasks_root`.
pub(crate) struct ReauthorRequest<'a> {
    pub(crate) project_root: &'a Path,
    pub(crate) tasks_root: &'a Path,
    pub(crate) prd_path: &'a Path,
    pub(crate) ids: &'a BTreeSet<String>,
    /// The executability probe every re-authored check must clear, and any
    /// finding already held per id.
    pub(crate) gate: ReauthorGate<'a>,
    /// What asked for the repair, recorded on the pin's lineage link.
    pub(crate) trigger: &'a str,
}

#[path = "workflow_acceptance_republish_result.rs"]
mod result;
pub(crate) use result::{CommittedRepair, ReauthorResult};
#[path = "workflow_acceptance_republish_verify.rs"]
pub(super) mod verify;
use verify::verify;
#[path = "workflow_acceptance_republish_attribute.rs"]
mod attribute;
#[path = "workflow_acceptance_republish_extend.rs"]
pub(crate) mod extend;

/// Re-author named checks and publish only if judgment, probes and frozen
/// gates pass and the verified chain still matches under the publication lock.
pub(crate) async fn reauthor_and_republish(
    client: &dyn WorkflowLlmClient,
    request: ReauthorRequest<'_>,
    scope: &AuthorScope,
) -> Result<ReauthorResult> {
    let verified = archon_workflow::stage_write::mapped(
        || {
            let _lock = ChainLock::acquire(
                &acceptance_pin_path(request.project_root, request.tasks_root),
                request.tasks_root,
            )?;
            verify(&request)
        },
        anyhow::Error::from,
    )?;
    // Re-judge with the model the freeze-time judge recorded, so a repaired
    // check is held to the same judge as every check kept as it was.
    let (judge_model, provider) = verified.judge.clone();
    match client.provider_id() {
        Some(actual) if actual == provider => {}
        actual => {
            return Err(anyhow!(
                "the freeze-time judge was {judge_model} on provider {provider}, but this client serves {}; a per-check repair must be judged like the checks it keeps — use that provider or re-run the whole-set freeze-acceptance",
                actual.as_deref().unwrap_or("an unreported provider")
            ));
        }
    }
    let state_dir = request.tasks_root.to_path_buf();
    let progress = reauthor::reauthor_partial_with_cache(
        client,
        &verified.contract,
        request.ids,
        scope,
        &judge_model,
        &request.gate,
        Some(&state_dir),
    )
    .await;
    let diagnostics = request.gate.probe.take_diagnostics();
    // Keep the typed stop reason as the error headline. Probe diagnostics are
    // separate log lines so an unproven check still ends incomplete (Issue 288).
    let mut progress = match progress {
        Ok(progress) => progress,
        Err(error) => {
            for diagnostic in diagnostics {
                eprintln!("{diagnostic}");
            }
            return Err(error);
        }
    };
    if request.gate.probe.baseline_commit().is_none() {
        for id in request.ids {
            let original = verified
                .contract
                .acceptance
                .iter()
                .chain(&verified.contract.supplementary)
                .find(|candidate| &candidate.id == id)
                .expect("verified request ids are present")
                .clone();
            if let Some(current) = progress
                .contract
                .acceptance
                .iter_mut()
                .chain(&mut progress.contract.supplementary)
                .find(|candidate| &candidate.id == id)
            {
                *current = original;
            }
            progress.unproven.entry(id.clone()).or_insert_with(|| {
                "the run has no pre-implementation tree to prove this check able to fail".into()
            });
            progress.authored_pending.remove(id);
        }
    }
    let mut still = non_accepted_ids(&progress.contract);
    still.retain(|id| !progress.unproven.contains_key(id));
    let unnamed: BTreeSet<_> = still.difference(request.ids).cloned().collect();
    if !unnamed.is_empty() {
        return Err(anyhow!(
            "{}",
            reauthor::not_accepted_report(&progress.contract, &unnamed, 0)
        ));
    }
    for id in still.intersection(request.ids) {
        let id = id.clone();
        let finding =
            reauthor::not_accepted_lines(&progress.contract, &BTreeSet::from([id.clone()]))
                .into_iter()
                .next()
                .unwrap_or_else(|| format!("check '{id}' remains unaccepted"));
        progress.unproven.insert(id, finding);
    }
    let mut publish_ids = request.ids.clone();
    for id in progress.unproven.keys() {
        publish_ids.remove(id);
    }
    // D4: a gate finding one candidate causes holds that candidate only.
    attribute::hold_back(&request, &verified, &mut progress, &mut publish_ids)?;
    if publish_ids.is_empty() {
        return Ok(ReauthorResult {
            acceptance_digest: verified.pin.acceptance_digest,
            freeze_event_id: verified.pin.freeze_event_id,
            skeleton_digest: verified.pin.skeleton_digest,
            diagnostics,
            unproven: progress.unproven,
            host_faults: progress.host_faults,
            authored_pending: progress.authored_pending,
            published: false,
        });
    }
    let published = publish_chain(
        &request,
        verified,
        progress.contract,
        diagnostics,
        None,
        &publish_ids,
    );
    let (mut result, settlement) = match published {
        Ok(result) => (result, None),
        Err(error) => match error.downcast_ref::<CommittedRepair>() {
            Some(committed) => (committed.result.clone(), Some(error)),
            None => return Err(error),
        },
    };
    result.unproven = progress.unproven;
    result.host_faults = progress.host_faults;
    result.authored_pending = progress.authored_pending;
    if let Some(error) = settlement {
        return Err(error.context(CommittedRepair { result }));
    }
    if result.unproven.is_empty() {
        reauthor::cache::archive(&state_dir, &result.acceptance_digest)
            .with_context(|| {
                format!(
                    "archiving consumed acceptance re-author candidates under {}",
                    state_dir.display()
                )
            })
            .with_context(|| CommittedRepair {
                result: result.clone(),
            })?;
    }
    Ok(result)
}

/// Publish `repaired` as the successor of the `verified` chain: gates in the
/// mode each stage was frozen in, the chain it replaces filed by digest, a
/// lineage link, one atomic transaction. `extension` (ACC-A7) adds checks and
/// may rebind the contract to the PRD as it is now (`republish_extend`).
fn publish_chain(
    request: &ReauthorRequest<'_>,
    verified: verify::Verified,
    repaired: AcceptanceContract,
    diagnostics: Vec<String>,
    extension: Option<&extend::Extension>,
    publish_ids: &BTreeSet<String>,
) -> Result<ReauthorResult> {
    archon_workflow::stage_write::mapped(
        || {
            let _lock = ChainLock::acquire(
                &acceptance_pin_path(request.project_root, request.tasks_root),
                request.tasks_root,
            )?;
            publish_chain_owned(
                request,
                verified,
                repaired,
                diagnostics,
                extension,
                publish_ids,
            )
        },
        anyhow::Error::from,
    )
}

// Caller holds the chain lock inside the stage write fence.
fn publish_chain_owned(
    request: &ReauthorRequest<'_>,
    verified: verify::Verified,
    repaired: AcceptanceContract,
    mut diagnostics: Vec<String>,
    extension: Option<&extend::Extension>,
    publish_ids: &BTreeSet<String>,
) -> Result<ReauthorResult> {
    let tasks_root = request.tasks_root;
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract_bytes = serde_json::to_vec_pretty(&repaired)?;
    let findings = findings::acceptance_findings(
        request.project_root,
        request.prd_path,
        &verified.prd_text,
        &contract_path,
        &repaired,
    )?;
    // M4: an extension answers for the PRD as it is now, so its gate holds
    // the coverage a whole-set freeze holds, whatever ids it publishes.
    let coverage = match extension {
        Some(_) => super::coverage_gate::acceptance_coverage_findings(
            &verified.prd_text,
            &contract_path,
            &repaired,
        ),
        None => Vec::new(),
    };
    // The freeze metadata and gate evaluate only the entries being published.
    // Findings for a still-pending refused entry stay in `findings` for the
    // returned repair report, while its non-accepted judgment remains on the
    // contract and therefore keeps the chain pending on the next read.
    let acceptance_gate_findings = findings
        .iter()
        .filter(|finding| {
            finding.remediation_scope == archon_workflow::RemediationScope::PrdInput
                || publish_ids.contains(&finding.subject)
        })
        .cloned()
        .chain(coverage)
        .collect::<Vec<_>>();
    let (lock, mut pin) = findings::acceptance_lock_and_pin(
        tasks_root,
        verified.acceptance_mode,
        &acceptance_gate_findings,
        &contract_bytes,
        request.gate.probe.baseline_commit(),
    );
    // Obligations are unchanged by a per-check repair, so the operator's
    // recorded waivers still describe this task set.
    pin.fidelity_waivers = verified.pin.fidelity_waivers.clone();
    let history = ChainHistory::for_pin(&verified.pin_path);
    // The chain this repair replaces stays provable: its contract and
    // skeleton are filed by digest before anything is published.
    history.put(&serde_json::to_vec_pretty(&verified.contract)?)?;
    if let Some(skeleton) = &verified.skeleton {
        history.put(&serde_json::to_vec_pretty(skeleton)?)?;
    }
    // PLAN-11: the re-authored checks' sources are re-pinned with the chain;
    // every other check keeps its pins.
    let sidecar = super::check_sources::frozen_sidecar(
        request.project_root,
        tasks_root,
        &contract_bytes,
        Some(publish_ids),
    )?;
    let sidecar_digest = content_digest(&sidecar.1);
    let mut files = vec![
        (contract_path, contract_bytes),
        (
            tasks_root.join(ACCEPTANCE_LOCK_FILE),
            serde_json::to_vec_pretty(&lock)?,
        ),
        sidecar,
    ];
    let mut gates = vec![(
        GateId::FreezeAcceptance,
        verified.acceptance_mode,
        acceptance_gate_findings,
    )];
    if let Some(mut skeleton) = verified.skeleton.clone() {
        if let Some(extension) = extension {
            extension.claim_new_obligations(&mut skeleton);
        }
        // The skeleton's content is unchanged; only the acceptance digest it
        // binds moves, so the skeleton gate re-runs over it here instead of in
        // a manual `freeze-skeleton` afterwards.
        skeleton
            .acceptance_digest
            .clone_from(&pin.acceptance_digest);
        validate_skeleton(&skeleton, &pin.acceptance_digest)?;
        let skeleton_findings = findings::skeleton_findings(
            tasks_root,
            &verified.canonical_prd,
            &verified.prd_text,
            &pin,
            &verified.pin_path,
            &skeleton,
        )?;
        let stamp = gate_stamp(verified.skeleton_mode, &skeleton_findings);
        let skeleton_bytes = serde_json::to_vec_pretty(&skeleton)?;
        let digest = content_digest(&skeleton_bytes);
        let skeleton_lock = TaskSkeletonLock {
            algorithm: "blake3".into(),
            digest: digest.clone(),
            acceptance_digest: pin.acceptance_digest.clone(),
            gate: stamp.clone(),
        };
        pin.skeleton_digest = Some(digest);
        pin.skeleton_gate = Some(stamp);
        files.push((tasks_root.join(TASK_SKELETON_FILE), skeleton_bytes));
        files.push((
            tasks_root.join(TASK_SKELETON_LOCK_FILE),
            serde_json::to_vec_pretty(&skeleton_lock)?,
        ));
        gates.push((
            GateId::FreezeSkeleton,
            verified.skeleton_mode,
            skeleton_findings,
        ));
    }
    pin.lineage = verified.pin.lineage.clone();
    let trigger = extension.map_or(request.trigger.to_string(), |extension| {
        format!("{}; {}", request.trigger, extension.note())
    });
    let link = PinTransition::extending(
        &pin.lineage,
        verified.pin.identity(),
        pin.identity(),
        publish_ids.iter().cloned().collect(),
        &trigger,
    );
    pin.lineage.push(link);
    // PLAN-11: the pin records the sidecar published beside it.
    pin.check_sources_digest = Some(sidecar_digest);
    files.push((verified.pin_path.clone(), serde_json::to_vec_pretty(&pin)?));
    let identity = content_digest(&serde_json::to_vec(
        &files
            .iter()
            .map(|(path, bytes)| (path.display().to_string(), content_digest(bytes)))
            .collect::<Vec<_>>(),
    )?);
    for (gate, frozen_mode, gate_findings) in gates {
        let mode = match frozen_mode {
            FreezeGateMode::Observe => GateMode::Observe,
            FreezeGateMode::Enforce => GateMode::Enforce,
        };
        let texts = gate_findings
            .iter()
            .map(|finding| finding.text.clone())
            .collect::<Vec<_>>();
        let mut disposition =
            crate::command::workflow_gate::run_sync_gate(request.project_root, mode, gate, || {
                Ok(
                    crate::command::workflow_gate::GateEvaluation::new("", gate_findings)
                        .with_publication_identity(identity.clone()),
                )
            })?;
        diagnostics.extend(disposition.diagnostics().iter().cloned());
        disposition.require_allowed().with_context(|| {
            format!(
                "the {} gate refused the per-check repair under {mode:?} mode, the mode that stage was frozen in; nothing was written",
                gate.as_str()
            )
        })?;
        let permit = disposition
            .take_publication_permit()
            .ok_or_else(|| anyhow!("per-check repair received no {} permit", gate.as_str()))?;
        if !permit.authorizes(gate, &texts, &identity) {
            return Err(anyhow!(
                "publication permit does not authorize the per-check repair"
            ));
        }
    }
    let transaction = begin_publish(
        &verified.pin_path,
        tasks_root,
        &files,
        "workflow freeze-acceptance --reauthor",
        verified.prior(),
    )?;
    // The published chain must verify exactly as every later reader verifies
    // it; if it does not, the prior chain is restored.
    let check = validate_acceptance_bundle(tasks_root, Some(&pin), &verified.expected)
        .map(|_| ())
        .map_err(|error| anyhow!("republished acceptance chain does not verify: {error}"))
        .and_then(|()| match verified.skeleton {
            Some(_) => validate_full_chain(tasks_root, &pin)
                .map(|_| ())
                .map_err(|error| anyhow!("republished skeleton chain does not verify: {error}")),
            None => Ok(()),
        });
    if let Err(error) = check {
        return Err(match transaction.roll_back() {
            Ok(()) => error.context("the prior chain was restored"),
            Err(rollback) => error.context(rollback.to_string()),
        });
    }
    let committed = transaction.commit();
    let mut result = ReauthorResult {
        acceptance_digest: pin.acceptance_digest,
        freeze_event_id: pin.freeze_event_id,
        skeleton_digest: pin.skeleton_digest,
        diagnostics,
        unproven: BTreeMap::new(),
        host_faults: BTreeMap::new(),
        authored_pending: BTreeMap::new(),
        published: true,
    };
    match committed {
        Ok(warnings) => result.diagnostics.extend(
            warnings
                .into_iter()
                .map(|warning| format!("warning: {warning}")),
        ),
        Err(error)
            if error
                .downcast_ref::<crate::command::workflow_host_command_committed::CommitRecorded>()
                .is_some() =>
        {
            return Err(error.context(CommittedRepair { result }));
        }
        Err(error) => return Err(error),
    }
    Ok(result)
}

#[path = "workflow_acceptance_republish_launch.rs"]
mod launch;
pub(crate) use launch::{reauthor_command, refuse_unaccepted_launch};

#[cfg(test)]
#[path = "workflow_acceptance_republish_test_fixture.rs"]
pub(crate) mod test_fixture;
#[cfg(test)]
#[path = "workflow_acceptance_republish_tests.rs"]
mod tests;
