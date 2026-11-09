//! Host-side diagnostic engine. No publication or scheduling authority.
use super::*;
use archon_workflow::baseline_results::{check_digest, results_digest};
use archon_workflow::breakage_proof::{selection, *};
use archon_workflow::task_set_contract::AcceptanceLock;
use archon_workflow::v2::requirement_classes::RequirementClassification;
#[path = "workflow_acceptance_executability_breakage_decide.rs"]
mod decide;
#[path = "workflow_acceptance_executability_breakage_group.rs"]
mod group;
#[path = "workflow_acceptance_executability_breakage_patch.rs"]
mod patch;
#[path = "workflow_acceptance_executability_breakage_run.rs"]
mod run;
#[path = "workflow_acceptance_executability_breakage_store.rs"]
mod store;
pub(crate) const MIN_DEADLINE_SECS: u64 = 60;
pub(crate) struct BreakageRequest<'a> {
    pub contract: &'a AcceptanceContract,
    pub chain_digest: &'a str,
    pub lock: &'a AcceptanceLock,
    pub classification: &'a RequirementClassification,
    pub mutants: &'a MutantSet,
    pub max_runs: u64,
    pub min_deadline_secs: u64,
}
pub(crate) struct BreakageReport {
    pub proofs: BreakageProofs,
    pub store: Option<PathBuf>,
}
pub(super) fn breakage_refusal(
    probe: &HostProbe,
    req: &BreakageRequest<'_>,
) -> Option<&'static str> {
    if probe.check_bound_secs() < 2 {
        return Some("invalid_time_cap");
    }
    let c = req.classification;
    if !matches!(probe.site, Site::Hermetic) {
        return Some("proof_site_unavailable");
    }
    if c.error.is_some() {
        return Some("classification_error");
    }
    let Some(snapshot) = &c.base_snapshot else {
        return Some("snapshot_missing");
    };
    if c.acceptance_digest != req.lock.digest
        || c.baseline_results_digest != results_digest(&req.lock.baseline_results)
        || !snapshot.valid_digest()
        || Some(&snapshot.commit) != req.lock.baseline_commit.as_ref()
    {
        return Some("classification_stale");
    }
    if probe.baseline_commit() != req.lock.baseline_commit {
        return Some("probe_baseline_mismatch");
    }
    None
}
async fn prepare(probe: &HostProbe, work: &Path, commit: &str) -> Result<(), RunOutcome> {
    let (parent, work, repository, commit) = (
        probe.copy_parent.clone(),
        work.to_path_buf(),
        probe.repository.clone(),
        commit.to_string(),
    );
    run::blocking(move || {
        base_copy::baseline_available(&repository, &commit).map_err(patch::host)?;
        std::fs::create_dir_all(&parent).map_err(|e| decide::unrun("host_io", &e.to_string()))?;
        owner::write(&work).map_err(patch::host)?;
        std::fs::create_dir_all(work.join("patches"))
            .map_err(|e| decide::unrun("host_io", &e.to_string()))?;
        std::fs::create_dir_all(work.join("build-output"))
            .map_err(|e| decide::unrun("host_io", &e.to_string()))?;
        let git = base_git::IsolatedGit::new(&work).map_err(patch::host)?;
        let transport = work.join("base.git");
        let format = base_copy::output(&repository, &["rev-parse", "--show-object-format"])
            .map_err(patch::host)?;
        let format = format!("--object-format={}", format.trim());
        git.run(&[
            "init".as_ref(),
            "--bare".as_ref(),
            "--quiet".as_ref(),
            format.as_ref(),
            transport.as_os_str(),
        ])
        .map_err(patch::host)?;
        let top = base_copy::output(&repository, &["rev-parse", "--show-toplevel"])
            .map_err(patch::host)?;
        let result = git
            .output(&[
                "-C".as_ref(),
                transport.as_os_str(),
                "fetch".as_ref(),
                "--no-tags".as_ref(),
                "--depth=1".as_ref(),
                top.trim().as_ref(),
                commit.as_ref(),
            ])
            .map_err(patch::host)?;
        if !result.status.success() {
            return Err(decide::unrun(
                "baseline_fetch_failed",
                &String::from_utf8_lossy(&result.stderr),
            ));
        }
        Ok(())
    })
    .await
}
pub(crate) async fn prove_breakage(probe: &HostProbe, req: BreakageRequest<'_>) -> BreakageReport {
    let mut proofs = BreakageProofs {
        schema: BREAKAGE_SCHEMA,
        base_commit: req.lock.baseline_commit.clone().unwrap_or_default(),
        controls: BTreeMap::new(),
        runs: BTreeMap::new(),
        summary: None,
    };
    let mut summary = ProofSummary {
        acceptance_digest: req.lock.digest.clone(),
        baseline_results_digest: results_digest(&req.lock.baseline_results),
        snapshot_digest: req
            .classification
            .base_snapshot
            .as_ref()
            .map(|s| s.digest.clone())
            .unwrap_or_default(),
        runs_executed: 0,
        runs_reused: 0,
        refusals: Vec::new(),
        entries: Vec::new(),
    };
    let (mutants, codes) = match selection::validate(req.mutants, req.lock, req.classification) {
        Ok(v) => v,
        Err(e) => (Vec::new(), vec![e.into()]),
    };
    summary.refusals.extend(codes);
    let mut groups: BTreeMap<String, Vec<SuppliedMutant>> = BTreeMap::new();
    for entry in &req.classification.entries {
        let checks = selection::passing_checks(entry, req.contract, req.lock);
        let countable = req
            .classification
            .base_snapshot
            .as_ref()
            .is_some_and(|s| !selection::keys(entry, s, &probe.repository, false).is_empty());
        let supplied: Vec<_> = mutants
            .iter()
            .filter(|m| m.task_id == entry.task_id && m.requirement_id == entry.id)
            .cloned()
            .collect();
        let state = if checks.is_empty() {
            ProofState::NoPassingCheck
        } else if !countable {
            ProofState::NoCountableDeliverable
        } else if supplied.is_empty() {
            ProofState::NoMutants
        } else {
            for id in &checks {
                groups
                    .entry(id.clone())
                    .or_default()
                    .extend(supplied.clone());
            }
            ProofState::NoViableMutant
        };
        summary.entries.push(RequirementProof {
            task_id: entry.task_id.clone(),
            id: entry.id.clone(),
            state,
            counted_kills: Vec::new(),
            checks,
        });
    }
    if let Some(code) = breakage_refusal(probe, &req) {
        summary.refusals.push(code.into());
    }
    let whole = summary.refusals.iter().any(|c| !c.starts_with("m-"));
    if whole {
        for pair in &mut summary.entries {
            if matches!(
                pair.state,
                ProofState::NoViableMutant | ProofState::NoMutants
            ) {
                pair.state = ProofState::HostUnrun;
            }
        }
    }
    let root = probe
        .resume
        .persist
        .then(|| store::root(&probe.project, &proofs.base_commit))
        .flatten();
    if let Some(path) = root
        .as_ref()
        .and_then(|r| store::summary_path(r, &req.lock.digest))
        && run::blocking(move || Ok(store::summary_unreadable(&path)))
            .await
            .unwrap_or(true)
    {
        summary.refusals.push("proofs_unreadable".into());
    }
    let work = probe
        .copy_parent
        .join(format!("{COPY_PREFIX}breakage-{}", uuid::Uuid::new_v4()));
    let parent = probe.copy_parent.clone();
    match run::blocking(move || Ok(sweep(&parent))).await {
        Ok(failures) => summary.refusals.extend(failures),
        Err(e) => summary.refusals.push(format!("{e:?}")),
    }
    if !whole && !groups.is_empty() {
        let prepared = prepare(probe, &work, &proofs.base_commit).await;
        for (id, mut mutants) in groups {
            mutants.sort_by_key(|m| (m.task_id.clone(), m.requirement_id.clone(), m.mutant_id()));
            group::execute(
                probe,
                &req,
                &id,
                mutants,
                &work,
                root.as_deref(),
                &prepared,
                &mut proofs,
                &mut summary,
            )
            .await;
        }
        let remove = work.clone();
        if let Err(e) =
            run::blocking(move || owner::remove(&remove).map_err(|e| decide::unrun("host_io", &e)))
                .await
        {
            summary
                .refusals
                .push(format!("work_dir_not_removed: {} ({e:?})", work.display()));
        }
    }
    for pair in &mut summary.entries {
        if pair.state != ProofState::NoViableMutant {
            continue;
        }
        let runs: Vec<_> = proofs
            .runs
            .values()
            .filter(|r| r.task_id == pair.task_id && r.requirement_id == pair.id)
            .collect();
        pair.counted_kills = runs
            .iter()
            .filter(|r| r.counted)
            .map(|r| r.mutant_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        pair.state = if runs
            .iter()
            .any(|r| r.counted && r.kind == MutantKind::Behaviour)
        {
            ProofState::BehaviourKill
        } else if runs.iter().any(|r| r.counted) {
            ProofState::BodyGutKillOnly
        } else if runs
            .iter()
            .any(|r| matches!(r.outcome, RunOutcome::BudgetExhausted))
        {
            ProofState::BudgetExhausted
        } else if runs.iter().any(|r| {
            matches!(
                r.outcome,
                RunOutcome::Unrun { ref code, .. } if code != "control_mismatch" && code != "zero_work"
            ) || matches!(r.outcome, RunOutcome::NoExitStatus)
        }) || proofs.controls.values().any(|c| {
            pair.checks.contains(&c.check_id)
                && [&c.pre, c.post.as_ref().unwrap_or(&c.pre)]
                    .into_iter()
                    .any(|o| {
                        matches!(o, RunOutcome::Unrun { code, .. } if code != "control_mismatch" && code != "zero_work")
                            || matches!(o, RunOutcome::NoExitStatus)
                    })
        }) {
            ProofState::HostUnrun
        } else if pair.checks.iter().all(|c| {
            !proofs
                .controls
                .values()
                .any(|r| &r.check_id == c && r.stable())
        }) {
            ProofState::Unverified
        } else if runs.iter().any(|r| {
            matches!(r.outcome, RunOutcome::Survived { .. })
                && !runs.iter().any(|other| {
                    other.mutant_id == r.mutant_id
                        && matches!(other.outcome, RunOutcome::Killed { .. })
                })
        }) {
            ProofState::SurvivorsOnly
        } else {
            ProofState::NoViableMutant
        };
    }
    proofs.summary = Some(summary);
    if let Some(path) = root
        .as_ref()
        .and_then(|r| store::summary_path(r, &req.lock.digest))
    {
        let mut saved = proofs.clone();
        saved.controls.retain(|_, control| control.stable());
        saved.runs.retain(|_, record| {
            store::reusable(&record.outcome)
                && store::has_evidence(record)
                && saved
                    .controls
                    .get(&record.control_key)
                    .is_some_and(|c| c.stable())
        });
        if run::blocking(move || {
            store::atomic(
                &path,
                &serde_json::to_vec_pretty(&saved)
                    .map_err(|e| decide::unrun("host_io", &e.to_string()))?,
            )
            .map_err(|e| decide::unrun("host_io", &e))
        })
        .await
        .is_err()
        {
            proofs
                .summary
                .as_mut()
                .expect("summary")
                .refusals
                .push("proofs_not_saved".into());
        }
    }
    BreakageReport {
        proofs,
        store: root,
    }
}
#[cfg(test)]
#[path = "workflow_acceptance_executability_breakage_decide_tests.rs"]
mod decide_tests;

#[cfg(test)]
#[path = "workflow_acceptance_executability_breakage_tests.rs"]
mod tests;

#[path = "workflow_acceptance_breaker.rs"]
pub(crate) mod breaker;
