//! Host-side diagnostic engine. No publication or scheduling authority.
use super::*;
use archon_workflow::baseline_results::{check_digest, results_digest};
use archon_workflow::breakage_proof::{selection, *};
use archon_workflow::task_set_contract::AcceptanceLock;
use archon_workflow::v2::requirement_classes::RequirementClassification;
#[path = "workflow_acceptance_executability_breakage_anchor.rs"]
pub(crate) mod anchor;
#[path = "workflow_acceptance_executability_breakage_decide.rs"]
mod decide;
use anchor::{Anchor, AnchoredPair};
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
    let validated = if req.lock.baseline_commit.is_none() && req.mutants.schema == BREAKAGE_SCHEMA {
        Err("mutant_commit_mismatch")
    } else {
        selection::validate(
            req.mutants,
            req.lock.baseline_commit.as_deref().unwrap_or_default(),
            &req.classification
                .entries
                .iter()
                .map(|e| (e.task_id.clone(), e.id.clone()))
                .collect::<Vec<_>>(),
        )
    };
    let (mutants, codes) = match validated {
        Ok(v) => v,
        Err(e) => (Vec::new(), vec![e.into()]),
    };
    summary.refusals.extend(codes);
    let mut pairs = Vec::new();
    for entry in &req.classification.entries {
        let checks = selection::passing_checks(entry, req.contract, req.lock);
        let countable = req
            .classification
            .base_snapshot
            .as_ref()
            .map(|s| selection::keys(entry, s, &probe.repository, false))
            .unwrap_or_default();
        let supplied: Vec<_> = mutants
            .iter()
            .filter(|m| m.task_id == entry.task_id && m.requirement_id == entry.id)
            .cloned()
            .collect();
        let state = if checks.is_empty() {
            ProofState::NoPassingCheck
        } else if countable.is_empty() {
            ProofState::NoCountableDeliverable
        } else if supplied.is_empty() {
            ProofState::NoMutants
        } else {
            ProofState::NoViableMutant
        };
        pairs.push(AnchoredPair {
            countable,
            shared: req
                .classification
                .base_snapshot
                .as_ref()
                .map(|s| selection::keys(entry, s, &probe.repository, true))
                .unwrap_or_default(),
            task_id: entry.task_id.clone(),
            id: entry.id.clone(),
            state: (state != ProofState::NoViableMutant).then_some(state),
            checks,
        });
    }
    if let Some(code) = breakage_refusal(probe, &req) {
        summary.refusals.push(code.into());
    }
    let whole = summary.refusals.iter().any(|c| !c.starts_with("m-"));
    if whole {
        for pair in &mut pairs {
            if pair.state.is_none() || pair.state == Some(ProofState::NoMutants) {
                pair.state = Some(ProofState::HostUnrun);
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
    if let Some(snapshot) = &req.classification.base_snapshot {
        let anchor = Anchor {
            contract: req.contract,
            chain_digest: req.chain_digest,
            commit: &snapshot.commit,
            snapshot,
            pairs,
            mutants,
            max_runs: req.max_runs,
            min_deadline_secs: req.min_deadline_secs,
            store_root: root.clone(),
        };
        let prior = std::mem::take(&mut summary.refusals);
        let report = if whole {
            anchor::execute(probe, anchor, prior).await
        } else {
            anchor::prove_anchored(probe, anchor, prior).await
        };
        proofs.controls = report.controls;
        proofs.runs = report.runs;
        summary.entries = report.entries;
        summary.refusals = report.refusals;
        summary.runs_executed = report.runs_executed;
        summary.runs_reused = report.runs_reused;
    } else {
        summary.entries = pairs.into_iter().map(AnchoredPair::proof).collect();
        anchor::sweep_copies(probe, &mut summary.refusals).await;
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
