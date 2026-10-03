//! The sanctioned repair of a frozen acceptance contract: re-author named
//! checks, keep every other entry byte-identical, and republish the whole
//! frozen chain — contract, lock, skeleton, skeleton lock and pin — in one
//! atomic transaction, so no follow-up `freeze-skeleton` and no hand edit of
//! the task directory is ever needed.
//!
//! Used by `workflow freeze-acceptance --reauthor` and by the authored run's
//! acceptance stage when a round meets a check the judge did not accept. Also
//! home of the launch guard that refuses to start a run bound to such a check.

use archon_workflow::task_set_contract::{
    ACCEPTANCE_LOCK_FILE, FreezeGateMode, TASK_SKELETON_LOCK_FILE,
};
use archon_workflow::task_set_lineage::{ChainHistory, PinTransition};
use archon_workflow::task_skeleton::validate_full_chain;

use super::reauthor::{AuthorScope, ReauthorGate};
use super::*;

/// One per-check repair of the contract frozen beside `tasks_root`.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReauthorResult {
    pub(crate) acceptance_digest: String,
    pub(crate) freeze_event_id: String,
    pub(crate) skeleton_digest: Option<String>,
    pub(crate) diagnostics: Vec<String>,
}

#[path = "workflow_acceptance_republish_verify.rs"]
pub(super) mod verify;
use verify::verify;
#[path = "workflow_acceptance_republish_extend.rs"]
pub(crate) mod extend;

/// Re-author and re-judge exactly `request.ids`, then republish the chain.
/// Nothing is written unless every named check ends accepted by the
/// freeze-time judge and cleared the executability probe, both gates allow publication under the mode their stage
/// was frozen in, and — checked inside the publish transaction, just before its
/// first rename — the chain on disk is still the one verified before the
/// author ran. The chain lock is held throughout.
pub(crate) async fn reauthor_and_republish(
    client: &dyn WorkflowLlmClient,
    request: ReauthorRequest<'_>,
    scope: &AuthorScope,
) -> Result<ReauthorResult> {
    let tasks_root = request.tasks_root;
    let _lock = ChainLock::acquire(
        &acceptance_pin_path(request.project_root, tasks_root),
        tasks_root,
    )?;
    let verified = verify(&request)?;
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
    let repaired = reauthor::reauthor(
        client,
        &verified.contract,
        request.ids,
        scope,
        &judge_model,
        &request.gate,
    )
    .await;
    let diagnostics = request.gate.probe.take_diagnostics();
    let repaired = repaired.map_err(|error| match diagnostics.is_empty() {
        true => error,
        false => anyhow!("{error:#}\nexecutability probe: {}", diagnostics.join("; ")),
    })?;
    let still = non_accepted_ids(&repaired);
    if !still.is_empty() {
        return Err(anyhow!(
            "{}",
            reauthor::not_accepted_report(&repaired, &still, reauthor::REAUTHOR_ATTEMPTS)
        ));
    }
    publish_chain(&request, verified, repaired, diagnostics, None)
}

/// Publish `repaired` as the successor of the `verified` chain: gates in the
/// mode each stage was frozen in, the chain it replaces filed by digest, a
/// lineage link, one atomic transaction. `extension` (ACC-A7) adds checks and
/// may rebind the contract to the PRD as it is now (`republish_extend`).
fn publish_chain(
    request: &ReauthorRequest<'_>,
    verified: verify::Verified,
    repaired: AcceptanceContract,
    mut diagnostics: Vec<String>,
    extension: Option<&extend::Extension>,
) -> Result<ReauthorResult> {
    let tasks_root = request.tasks_root;
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract_bytes = serde_json::to_vec_pretty(&repaired)?;
    let mut findings = findings::acceptance_findings(
        request.prd_path,
        &verified.prd_text,
        &contract_path,
        &repaired,
    );
    // M4: an extension answers for the PRD as it is now, so its gate holds
    // the coverage a whole-set freeze holds: every requirement covered.
    if extension.is_some() {
        findings.extend(super::coverage_gate::acceptance_coverage_findings(
            &verified.prd_text,
            &contract_path,
            &repaired,
        ));
    }
    let (lock, mut pin) = findings::acceptance_lock_and_pin(
        tasks_root,
        verified.acceptance_mode,
        &findings,
        &contract_bytes,
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
        Some(request.ids),
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
    let mut gates = vec![(GateId::FreezeAcceptance, verified.acceptance_mode, findings)];
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
        request.ids.clone(),
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
    for warning in transaction.commit()? {
        diagnostics.push(format!("warning: {warning}"));
    }
    Ok(ReauthorResult {
        acceptance_digest: pin.acceptance_digest,
        freeze_event_id: pin.freeze_event_id,
        skeleton_digest: pin.skeleton_digest,
        diagnostics,
    })
}

/// The exact operator command that repairs `ids` in the contract beside
/// `tasks_root`.
pub(crate) fn reauthor_command(
    project_root: &Path,
    tasks_root: &Path,
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
) -> String {
    let prd = PathBuf::from(&contract.prd.path);
    let prd = if prd.is_absolute() {
        prd
    } else {
        project_root.join(prd)
    };
    format!(
        "cd {} && archon workflow freeze-acceptance {} --tasks {} --prd {}",
        project_root.display(),
        ids.iter()
            .map(|id| format!("--reauthor {id}"))
            .collect::<Vec<_>>()
            .join(" "),
        tasks_root.display(),
        prd.display()
    )
}

/// Refuse to launch a run bound to a contract carrying any check the judge
/// did not accept (such a check can never pass, so the run could never
/// complete), or bound to a freeze the host cannot read back: an unreadable
/// contract, or a lock whose pin is missing or unreadable. Any publish of the
/// set a crash interrupted is first settled to one whole version (Issue 271).
/// A task set with no contract and no lock launches.
pub(crate) fn refuse_unaccepted_launch(project_root: &Path, tasks_root: &Path) -> Result<()> {
    recover_interrupted_publish(&acceptance_pin_path(project_root, tasks_root), tasks_root)?;
    let path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let locked = tasks_root.join(ACCEPTANCE_LOCK_FILE).exists();
    if !path.exists() && !locked {
        return Ok(());
    }
    let contract: AcceptanceContract = std::fs::read(&path)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(anyhow::Error::from))
        .with_context(|| {
            format!(
                "refusing to launch: the bound acceptance contract {} cannot be read; restore it or re-freeze",
                path.display()
            )
        })?;
    if locked {
        let pin_path = acceptance_pin_path(project_root, tasks_root);
        std::fs::read(&pin_path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| {
                serde_json::from_slice::<AcceptancePin>(&bytes).map_err(anyhow::Error::from)
            })
            .with_context(|| {
                format!(
                    "refusing to launch: {} is frozen but its pin {} cannot be read; restore it or re-freeze",
                    path.display(),
                    pin_path.display()
                )
            })?;
    }
    let bad = non_accepted_ids(&contract);
    if bad.is_empty() {
        return Ok(());
    }
    Err(anyhow!(
        "refusing to launch: {} is bound to this run and carries {} check(s) the judge did not accept, which can never pass:\n  - {}\nRepair only those entries (the skeleton chain is re-bound in the same step), then launch again:\n  {}",
        path.display(),
        bad.len(),
        reauthor::not_accepted_lines(&contract, &bad).join("\n  - "),
        reauthor_command(project_root, tasks_root, &contract, &bad)
    ))
}

#[cfg(test)]
#[path = "workflow_acceptance_republish_test_fixture.rs"]
pub(crate) mod test_fixture;
#[cfg(test)]
#[path = "workflow_acceptance_republish_tests.rs"]
mod tests;
