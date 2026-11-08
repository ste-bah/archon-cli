//! Gate findings and pin/lock assembly shared by every acceptance publication:
//! the whole-set freeze, the per-check re-author, and the in-run repair.

use super::*;
pub(super) use crate::command::workflow_task_set_classification::{
    baseline_results, carry_baseline_results, classification, classification_from_lock,
};
use archon_workflow::task_set_contract::JudgeDecision;
use std::collections::BTreeMap;

#[path = "workflow_task_set_acceptance_findings.rs"]
mod acceptance;
pub(super) use acceptance::acceptance_findings;

/// Ids of every check whose judge verdict is not `accepted`.
pub(crate) fn non_accepted_ids(contract: &AcceptanceContract) -> BTreeSet<String> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .filter(|entry| entry.judgment.verdict != JudgeDecision::Accepted)
        .map(|entry| entry.id.clone())
        .collect()
}

/// The expansion gate has repairable findings even for a stored accepted verdict.
/// Keep the exact finding as the re-author's input.
pub(crate) fn unset_check_findings(
    contract: &AcceptanceContract,
    bound: &[&str],
) -> BTreeMap<String, String> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .filter_map(|entry| {
            let text = match &entry.check {
                archon_workflow::task_set_contract::AcceptanceCheck::Command {
                    command, ..
                } => Some(command.as_str()),
                archon_workflow::task_set_contract::AcceptanceCheck::Floor { contract } => {
                    contract.typed_verifier_command.as_deref()
                }
            }?;
            let findings =
                archon_workflow::acceptance_check_shell_unset::unset_reads_with_bound(text, bound);
            (!findings.is_empty()).then(|| {
                (
                    entry.id.clone(),
                    findings
                        .iter()
                        .map(|read| read.finding())
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })
        })
        .collect()
}

/// Stamp, lock and pin a judged contract into a publishable freeze; the
/// lock records `baseline_commit`, the tree its checks were proven on.
#[allow(clippy::too_many_arguments)]
pub(super) fn finish_acceptance(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    prd_text: &str,
    freeze_mode: FreezeGateMode,
    contract: &AcceptanceContract,
    probed: Vec<GateFinding>,
    mut baseline: archon_workflow::baseline_results::BaselineResults,
    candidate_digest: String,
) -> Result<PreparedAcceptanceFreeze> {
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let mut findings =
        acceptance_findings(project_root, prd_path, prd_text, &contract_path, contract)?;
    // #549: a check the host did not judge (its own named files exceed the
    // judged-source limit of one check) goes back to its author with what it
    // names and the limit, never as a refutation of content nobody saw.
    let not_judged = judge::not_judged_findings(&contract_path, contract);
    findings.retain(|finding| {
        !(not_judged.iter().any(|own| own.subject == finding.subject)
            && (finding.text).starts_with(&format!("check '{}' was refuted", finding.subject)))
    });
    findings.extend(not_judged);
    // Issue 275: a check refuted because it cannot pass as written has its
    // own finding, carrying its pre-implementation output; the generic
    // refutation would only tell its author to make it fail.
    let cannot_pass: BTreeSet<&str> = (probed.iter())
        .filter(|finding| is_baseline_pass_finding(finding))
        .map(|finding| finding.subject.as_str())
        .collect();
    findings.retain(|finding| {
        !(cannot_pass.contains(finding.subject.as_str())
            && (finding.text).starts_with(&format!("check '{}' was refuted", finding.subject)))
    });
    // Setup and passability refusals are not product verdicts.
    baseline.outcomes.retain(|id, result| {
        !(cannot_pass.contains(id.as_str())
            && result.outcome == archon_workflow::baseline_results::BaselineOutcome::Failed)
    });
    // What the executability probe found on the pre-implementation tree.
    findings.extend(probed);
    // H4: a whole-set freeze is not finished while a requirement is covered
    // by no check; each goes back to the author as the check it is owed.
    findings.extend(super::coverage_gate::acceptance_coverage_findings(
        prd_text,
        &contract_path,
        contract,
    ));
    let contract_bytes = serde_json::to_vec_pretty(contract)?;
    let (lock, pin) = acceptance_lock_and_pin(
        tasks_root,
        freeze_mode,
        &findings,
        &contract_bytes,
        baseline,
    );
    Ok(PreparedAcceptanceFreeze {
        tasks_root: tasks_root.to_path_buf(),
        project_root: project_root.to_path_buf(),
        non_accepted: non_accepted_ids(contract),
        result: FreezeAcceptanceResult {
            acceptance_digest: pin.acceptance_digest.clone(),
            freeze_event_id: pin.freeze_event_id.clone(),
            candidate_digest: candidate_digest.clone(),
            live_contract_digest: None,
        },
        contract_bytes,
        candidate_digest,
        live_contract_digest: None,
        recovery: None,
        lock,
        pin,
        findings,
    })
}

pub(super) fn is_baseline_pass_finding(finding: &GateFinding) -> bool {
    let current = finding
        .text
        .starts_with(&format!("check '{}': it passed on the ", finding.subject))
        && finding.text.contains(super::executability::CANNOT_FAIL);
    let legacy = "passed on the pre-implementation tree";
    let selected = format!(
        "check '{}': it passed on the selected tree at ",
        finding.subject
    );
    finding
        .text
        .starts_with(&cannot_pass_prefix(&finding.subject))
        || current
        || finding.text.starts_with(&selected)
        || ((finding.text.starts_with(&format!(
            "check '{}': the criterion already holds on the pre-implementation tree;",
            finding.subject
        )) || finding
            .text
            .starts_with(&format!("check '{}': it {legacy} at ", finding.subject)))
            && finding.text.contains(legacy))
}

/// The passability-specific finding added to an ordinary judge refusal.
fn cannot_pass_prefix(id: &str) -> String {
    format!("check '{id}': {}", super::passability::CANNOT_PASS)
}

/// A fresh acceptance lock and a pin with no successor skeleton bound. The
/// lock records `baseline_commit`, the tree this publication's checks were
/// proven on -- which is the recorded one whenever the task set's lock
/// records one (`Baseline::for_task_set`, `Baseline::for_round`) -- else
/// the baseline the lock already records (Issue 328: one baseline).
pub(super) fn acceptance_lock_and_pin(
    tasks_root: &Path,
    freeze_mode: FreezeGateMode,
    findings: &[GateFinding],
    contract_bytes: &[u8],
    baseline: archon_workflow::baseline_results::BaselineResults,
) -> (AcceptanceLock, AcceptancePin) {
    let stamp = gate_stamp(freeze_mode, findings);
    let digest = content_digest(contract_bytes);
    let commit = baseline
        .commit
        .clone()
        .or_else(|| super::executability::recorded_commit(tasks_root));
    let baseline_results = if commit.is_some() && baseline.commit == commit {
        serde_json::from_slice::<AcceptanceContract>(contract_bytes)
            .map(|contract| baseline.current(&contract))
            .expect("serialized acceptance contract is valid")
    } else {
        BTreeMap::new()
    };
    let lock = AcceptanceLock {
        baseline_results,
        algorithm: "blake3".into(),
        digest: digest.clone(),
        gate: stamp.clone(),
        baseline_commit: commit,
    };
    let pin = AcceptancePin {
        check_sources_digest: None,
        task_root: archon_shell::paths::stored_text(
            &tasks_root
                .canonicalize()
                .map(archon_shell::paths::plain)
                .unwrap_or_else(|_| tasks_root.to_path_buf()),
        ),
        acceptance_digest: digest.clone(),
        freeze_event_id: format!("acceptance-freeze-{}", &digest[..12]),
        acceptance_gate: stamp,
        skeleton_digest: None,
        skeleton_gate: None,
        fidelity_waivers: Vec::new(),
        lineage: Vec::new(),
        lineage_recording: Some(archon_workflow::task_set_lineage::LINEAGE_RECORDING_V1),
    };
    (lock, pin)
}

#[path = "workflow_task_set_skeleton_findings.rs"]
mod skeleton;
pub(super) use skeleton::skeleton_findings;

impl PreparedAcceptanceFreeze {
    /// Checks in this freeze the judge did not accept.
    pub(crate) fn non_accepted_ids(&self) -> &BTreeSet<String> {
        &self.non_accepted
    }

    /// The judged contract this freeze would publish.
    pub(crate) fn contract(&self) -> Result<AcceptanceContract> {
        serde_json::from_slice(&self.contract_bytes).context("parsing the prepared contract")
    }

    /// Refuse publication of any check the judge did not accept: such a check
    /// can never run, so publishing it only schedules work that cannot pass.
    pub(crate) fn require_all_accepted(&self) -> Result<()> {
        if self.non_accepted.is_empty() {
            return Ok(());
        }
        let contract = self.contract()?;
        Err(anyhow!(
            "{}",
            super::reauthor::not_accepted_report(&contract, &self.non_accepted, 0)
        ))
    }
}

#[path = "workflow_task_set_findings_prepare.rs"]
mod prepare;
pub(crate) use prepare::{capture_live_contract_digest, prepare_from_judged};

/// #552: a later step of this freeze could not complete. The checks it
/// already knew it could not judge (`entries`) keep their own findings in
/// that outcome, which names them, so each reaches its own author in the same
/// round instead of only the checks the step left without a verdict.
pub(super) fn with_known_findings<'a>(
    mut error: anyhow::Error,
    tasks_root: &Path,
    entries: impl IntoIterator<Item = &'a archon_workflow::task_set_contract::AcceptanceCriterion>,
) -> anyhow::Error {
    if let Some(incomplete) =
        error.downcast_mut::<crate::command::workflow_freeze_budget::FreezeIncomplete>()
    {
        incomplete.add_known_findings(judge::not_judged_findings_of(
            &tasks_root.join(ACCEPTANCE_CONTRACT_FILE),
            entries,
        ));
    }
    error
}

/// Prepare a whole-set freeze, returning every check the judge did not accept
/// to its author — with the judge's reason and counterexample — bounded, before
/// anything can publish. A check still not accepted fails the freeze.
#[cfg(test)]
pub(crate) async fn prepare_acceptance_freeze_reauthoring(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    client: Arc<dyn WorkflowLlmClient>,
    scope: &super::reauthor::AuthorScope,
) -> Result<PreparedAcceptanceFreeze> {
    prepare_acceptance_freeze_reauthoring_with_limits(
        project_root,
        tasks_root,
        prd_path,
        mode,
        client,
        scope,
        archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    )
    .await
}

pub(crate) async fn prepare_acceptance_freeze_reauthoring_with_limits(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    client: Arc<dyn WorkflowLlmClient>,
    scope: &super::reauthor::AuthorScope,
    resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
) -> Result<PreparedAcceptanceFreeze> {
    use super::executability::ExecutabilityProbe;
    let prepared = prepare_acceptance_freeze_with_limits(
        project_root,
        tasks_root,
        prd_path,
        mode,
        client.clone(),
        resource_limits.clone(),
    )
    .await?;
    let contract = prepared.contract()?;
    // A newly authored check the judge accepted is run once in a hermetic
    // copy (the scratch site, else the probe's own) before it may be
    // published: one that crashes in its own code goes back to its author
    // with the crash. A4: the freeze-time probe also proves each check can
    // fail on the task set's pre-implementation tree; one that cannot is
    // re-authored like a crash. Verdicts the freeze's own gate already
    // observed on the same tree are reused, not run again.
    let probe = super::executability::HostProbe::for_task_set_with_limits(
        project_root,
        tasks_root,
        resource_limits,
    );
    let accepted: BTreeSet<String> = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
        .map(|entry| entry.id.clone())
        .collect();
    let crashed = probe.script_defects(&contract, &accepted).await;
    // A check the host could not prove, even after repairing its own
    // environment, is the host's: nothing is published and no author is
    // asked to change it. Like the staged freeze (Issue 263), it ends the
    // freeze incomplete and resumable, never failed (Issue 288). Results are
    // per check (as a keep-going build keeps them): a defect another
    // check's own run showed still goes to its author first.
    let unproven = probe.take_unproven();
    let host_unproven = |unproven| {
        for diagnostic in probe.take_diagnostics() {
            eprintln!("{diagnostic}");
        }
        Err(unproven_incomplete(
            super::executability::HostUnproven(unproven).into(),
        ))
    };
    let mut named = prepared.non_accepted_ids().clone();
    named.extend(crashed.keys().cloned());
    if !unproven.is_empty() && named.is_empty() {
        return host_unproven(unproven);
    }
    // A check that cannot pass as written is first shown its own finding,
    // with its pre-implementation output (Issue 275).
    let mut seeds = crashed.clone();
    super::reauthor::seed_findings(&mut seeds, &named, &prepared.findings);
    if named.is_empty() {
        for diagnostic in probe.take_diagnostics() {
            eprintln!("{diagnostic}");
        }
        return Ok(prepared);
    }
    eprintln!(
        "{} check(s) not accepted by the judge, crashing in their own code, or not shown able to fail; re-authoring each until {} consecutive attempts repair none: {}",
        named.len(),
        super::reauthor::REAUTHOR_ATTEMPTS,
        named.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    // A pass that ends incomplete for an unproven check saves each repair
    // that clears every gate under the task root (`reauthor::cache`), and
    // the attempt after it reuses them without asking an author or the
    // judge again; a pass that can publish keeps nothing.
    let gate = super::reauthor::ReauthorGate {
        probe: &probe,
        seeds: &seeds,
    };
    let kept = !unproven.is_empty() || super::reauthor::cache::saved(tasks_root);
    let repaired = match kept {
        true => {
            super::reauthor::reauthor_with_cache(
                client.as_ref(),
                &contract,
                &named,
                scope,
                &client.model_for_role("judge", super::judge::JUDGE_MODEL_ALIAS),
                &gate,
                Some(tasks_root),
            )
            .await
        }
        false => {
            let judge = &client.model_for_role("judge", super::judge::JUDGE_MODEL_ALIAS);
            super::reauthor::reauthor(client.as_ref(), &contract, &named, scope, judge, &gate).await
        }
    };
    for diagnostic in probe.take_diagnostics() {
        eprintln!("{diagnostic}");
    }
    let repaired = repaired.map_err(unproven_incomplete)?;
    if !unproven.is_empty() {
        return host_unproven(unproven);
    }
    let baseline = archon_workflow::baseline_results::BaselineResults {
        commit: prepared.lock.baseline_commit.clone(),
        outcomes: prepared
            .lock
            .baseline_results
            .into_iter()
            .filter(|(id, _)| !named.contains(id))
            .collect(),
    };
    let live_contract_digest = prepared.live_contract_digest.clone();
    let prepared = prepare::prepare_from_judged_with_results(
        project_root,
        tasks_root,
        prd_path,
        mode,
        &repaired,
        Some(baseline),
        None,
        live_contract_digest,
    )?;
    // Every saved repair is in the prepared contract now.
    if kept {
        super::reauthor::cache::archive(tasks_root, &prepared.result.acceptance_digest)?;
    }
    Ok(prepared)
}

/// A check the host could not prove, or a re-author that stopped making
/// progress, ends the unstaged freeze incomplete and resumable, never failed
/// (Issue 288). The reason is the whole report, every context included (the
/// probe's diagnostics, the pending checks' last findings); any other error
/// is returned unchanged.
pub(crate) fn unproven_incomplete(error: anyhow::Error) -> anyhow::Error {
    let stalled = error.chain().any(|cause| {
        cause.is::<super::executability::HostUnproven>()
            || cause.is::<super::reauthor::ReauthorStalled>()
    });
    match stalled {
        true => {
            crate::command::workflow_freeze_budget::FreezeIncomplete::unsaved(format!("{error:#}"))
                .into()
        }
        false => error,
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_task_set_findings_tests.rs"]
pub(super) mod tests;
