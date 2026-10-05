//! Gate findings and pin/lock assembly shared by every acceptance publication:
//! the whole-set freeze, the per-check re-author, and the in-run repair.

use super::*;
use archon_workflow::task_set_contract::JudgeDecision;

/// Every finding the acceptance gate records for `contract` against the PRD.
pub(super) fn acceptance_findings(
    prd_path: &Path,
    prd_text: &str,
    contract_path: &Path,
    contract: &AcceptanceContract,
) -> Vec<GateFinding> {
    let mut findings = malformed_obligation_ids(prd_text)
        .into_iter()
        .map(|id| {
            GateFinding::new(
                GateId::FreezeAcceptance,
                malformed_obligation_finding(&id),
                id,
                Some(prd_path.to_path_buf()),
                archon_workflow::RemediationScope::PrdInput,
            )
        })
        .collect::<Vec<_>>();
    findings.extend(duplicate_obligation_ids(prd_text).into_iter().map(|id| {
        GateFinding::new(
            GateId::FreezeAcceptance,
            duplicate_obligation_finding(&id),
            id,
            Some(prd_path.to_path_buf()),
            archon_workflow::RemediationScope::PrdInput,
        )
    }));
    findings.extend(
        acceptance_policy_findings(contract)
            .into_iter()
            .map(|finding| {
                let subject = finding
                    .field
                    .split('.')
                    .next()
                    .unwrap_or("acceptance-contract");
                let entry = contract
                    .acceptance
                    .iter()
                    .chain(&contract.supplementary)
                    .find(|criterion| criterion.id == subject);
                // Repairable unless the PRD itself prescribed the check's shape
                // (TD-015): outcome language leaves the shape to the author, so a
                // floor that cannot fail goes back with the finding; a criterion
                // naming the contract fields has fixed the shape, so that
                // finding is recorded, never retried, never blocking in observe.
                let prescribed = entry.is_some_and(|criterion| {
                    criterion_prescribes_check_shape(&criterion.criterion)
                });
                // A judge verdict other than `accepted` is never an observation:
                // such a check can never run, so it is never published, and the
                // only way forward is the author's. It goes back to the author
                // whatever the criterion prescribes.
                let refuted = finding.field.ends_with(".judgment")
                    && entry.is_some_and(|criterion| {
                        criterion.judgment.verdict != JudgeDecision::Accepted
                    });
                let scope = if prescribed && !refuted {
                    archon_workflow::RemediationScope::InheritedPredecessor
                } else {
                    archon_workflow::RemediationScope::CandidateArtifact
                };
                let mut gate_finding = GateFinding::new(
                    GateId::FreezeAcceptance,
                    finding.message,
                    subject,
                    Some(contract_path.to_path_buf()),
                    scope,
                );
                gate_finding.deterministic_defect = finding.identity;
                gate_finding
            }),
    );
    findings
}

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
    baseline_commit: Option<String>,
) -> Result<PreparedAcceptanceFreeze> {
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let mut findings = acceptance_findings(prd_path, prd_text, &contract_path, contract);
    // Issue 275: a check refuted because it cannot pass as written has its
    // own finding, carrying its pre-implementation output; the generic
    // refutation would only tell its author to make it fail.
    let cannot_pass: BTreeSet<&str> = (probed.iter())
        .filter(|finding| {
            finding
                .text
                .starts_with(&cannot_pass_prefix(&finding.subject))
        })
        .map(|finding| finding.subject.as_str())
        .collect();
    findings.retain(|finding| {
        !(cannot_pass.contains(finding.subject.as_str())
            && (finding.text).starts_with(&format!("check '{}' was refuted", finding.subject)))
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
        baseline_commit,
    );
    Ok(PreparedAcceptanceFreeze {
        tasks_root: tasks_root.to_path_buf(),
        project_root: project_root.to_path_buf(),
        non_accepted: non_accepted_ids(contract),
        result: FreezeAcceptanceResult {
            acceptance_digest: pin.acceptance_digest.clone(),
            freeze_event_id: pin.freeze_event_id.clone(),
        },
        contract_bytes,
        recovery: None,
        lock,
        pin,
        findings,
    })
}

/// How the finding for check `id` that cannot pass as written starts.
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
    baseline_commit: Option<String>,
) -> (AcceptanceLock, AcceptancePin) {
    let stamp = gate_stamp(freeze_mode, findings);
    let digest = content_digest(contract_bytes);
    let lock = AcceptanceLock {
        algorithm: "blake3".into(),
        digest: digest.clone(),
        gate: stamp.clone(),
        baseline_commit: baseline_commit
            .or_else(|| super::executability::recorded_commit(tasks_root)),
    };
    let pin = AcceptancePin {
        check_sources_digest: None,
        task_root: tasks_root
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap_or_else(|_| tasks_root.to_path_buf())
            .display()
            .to_string(),
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

/// Every finding the skeleton gate records for `skeleton` under `pin`.
pub(super) fn skeleton_findings(
    tasks_root: &Path,
    canonical_prd: &Path,
    prd_text: &str,
    pin: &AcceptancePin,
    pin_path: &Path,
    skeleton: &TaskSkeleton,
) -> Result<Vec<GateFinding>> {
    let skeleton_path = tasks_root.join(TASK_SKELETON_FILE);
    let mut findings = malformed_obligation_ids(prd_text)
        .into_iter()
        .map(|id| {
            GateFinding::new(
                GateId::FreezeSkeleton,
                malformed_obligation_finding(&id),
                id,
                Some(canonical_prd.to_path_buf()),
                archon_workflow::RemediationScope::PrdInput,
            )
        })
        .collect::<Vec<_>>();
    findings.extend(duplicate_obligation_ids(prd_text).into_iter().map(|id| {
        GateFinding::new(
            GateId::FreezeSkeleton,
            duplicate_obligation_finding(&id),
            id,
            Some(canonical_prd.to_path_buf()),
            archon_workflow::RemediationScope::PrdInput,
        )
    }));
    predecessor_findings(pin, pin_path, &mut findings);
    let expected_obligations = archon_workflow::obligation_ids::obligation_ids(prd_text);
    findings.extend(
        validate_skeleton_set(skeleton, &expected_obligations)
            .into_iter()
            .map(|finding| {
                GateFinding::new(
                    GateId::FreezeSkeleton,
                    format!("{}: {}", finding.field, finding.message),
                    finding.field,
                    Some(skeleton_path.clone()),
                    archon_workflow::RemediationScope::Skeleton,
                )
                .with_defect(finding.identity)
            }),
    );
    let edge_analysis = analyze_task_set_edges(skeleton);
    findings.extend(edge_analysis.blockers.into_iter().map(|finding| {
        GateFinding::new(
            GateId::FreezeSkeleton,
            format!("{}: {}", finding.field, finding.message),
            finding.field,
            Some(skeleton_path.clone()),
            archon_workflow::RemediationScope::Skeleton,
        )
        .with_defect(finding.identity)
    }));
    // Issue-55: every repository file the PRD names must have an owning
    // task; the skeleton author assigns it on the retry this finding drives.
    findings.extend(
        crate::command::topology_lint::skeleton_owner_defects(tasks_root, prd_text, skeleton)?
            .into_iter()
            .map(|defect| {
                GateFinding::new(
                    GateId::FreezeSkeleton,
                    defect.message,
                    "deliverable_contracts",
                    Some(skeleton_path.clone()),
                    archon_workflow::RemediationScope::Skeleton,
                )
                .with_defect(defect.identity)
            }),
    );
    Ok(findings)
}

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

/// Re-prepare a freeze from a contract whose checks were re-judged in place
/// and proven on `baseline_commit`.
pub(crate) fn prepare_from_judged(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    contract: &AcceptanceContract,
    baseline_commit: Option<String>,
) -> Result<PreparedAcceptanceFreeze> {
    let (prd, _, _) = validate_prd_input(prd_path)?;
    let prd_text = String::from_utf8(prd).context("PRD is not UTF-8")?;
    finish_acceptance(
        project_root,
        tasks_root,
        prd_path,
        &prd_text,
        freeze_mode(mode)?,
        contract,
        Vec::new(),
        baseline_commit,
    )
}

/// Prepare a whole-set freeze, returning every check the judge did not accept
/// to its author — with the judge's reason and counterexample — bounded, before
/// anything can publish. A check still not accepted fails the freeze.
pub(crate) async fn prepare_acceptance_freeze_reauthoring(
    project_root: &Path,
    tasks_root: &Path,
    prd_path: &Path,
    mode: GateMode,
    client: Arc<dyn WorkflowLlmClient>,
    scope: &super::reauthor::AuthorScope,
) -> Result<PreparedAcceptanceFreeze> {
    use super::executability::ExecutabilityProbe;
    let prepared =
        prepare_acceptance_freeze(project_root, tasks_root, prd_path, mode, client.clone()).await?;
    let contract = prepared.contract()?;
    // A newly authored check the judge accepted is run once in a hermetic
    // copy (the scratch site, else the probe's own) before it may be
    // published: one that crashes in its own code goes back to its author
    // with the crash. A4: the freeze-time probe also proves each check can
    // fail on the task set's pre-implementation tree; one that cannot is
    // re-authored like a crash. Verdicts the freeze's own gate already
    // observed on the same tree are reused, not run again.
    let probe = super::executability::HostProbe::for_task_set(project_root, tasks_root);
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
    // freeze incomplete and resumable, never failed (Issue 288).
    let unproven = probe.take_unproven();
    if !unproven.is_empty() {
        for diagnostic in probe.take_diagnostics() {
            eprintln!("{diagnostic}");
        }
        return Err(unproven_incomplete(
            super::executability::HostUnproven(unproven).into(),
        ));
    }
    let mut named = prepared.non_accepted_ids().clone();
    named.extend(crashed.keys().cloned());
    // A check that cannot pass as written is first shown its own finding,
    // with its pre-implementation output (Issue 275).
    let mut seeds = crashed.clone();
    for finding in &prepared.findings {
        if finding
            .text
            .starts_with(&cannot_pass_prefix(&finding.subject))
        {
            seeds.insert(finding.subject.clone(), finding.text.clone());
        }
    }
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
    let repaired = super::reauthor::reauthor(
        client.as_ref(),
        &contract,
        &named,
        scope,
        "sonnet",
        &super::reauthor::ReauthorGate {
            probe: &probe,
            seeds: &seeds,
        },
    )
    .await;
    for diagnostic in probe.take_diagnostics() {
        eprintln!("{diagnostic}");
    }
    let repaired = repaired.map_err(unproven_incomplete)?;
    let baseline = prepared.lock.baseline_commit.clone();
    prepare_from_judged(
        project_root,
        tasks_root,
        prd_path,
        mode,
        &repaired,
        baseline,
    )
}

/// An unproven check ends the unstaged freeze incomplete and resumable
/// (Issue 288), with the host's own report as its reason; any other error is
/// returned unchanged.
pub(crate) fn unproven_incomplete(error: anyhow::Error) -> anyhow::Error {
    let unproven = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<super::executability::HostUnproven>());
    match unproven {
        Some(unproven) => {
            crate::command::workflow_freeze_budget::FreezeIncomplete::unsaved(unproven.to_string())
                .into()
        }
        None => error,
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_task_set_findings_tests.rs"]
pub(super) mod tests;
