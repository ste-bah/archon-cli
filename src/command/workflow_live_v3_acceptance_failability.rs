//! Issue 219 (A5 at run time): a check that passes a round must be proven
//! able to fail.
//!
//! A freeze probes every check it publishes on the pre-implementation tree,
//! and every check authored or repaired in a run is probed the same way. A
//! frozen chain that reaches a round without that proof (a chain frozen
//! before the probe existed, or one whose proof was never recorded) used to
//! run as it stood: a check that passes on any tree -- a replay compared
//! with itself -- "passed" and the run was Accepted on it.
//!
//! So every check that PASSES a round is held to the tree before
//! implementation -- the run's base commit, else the one the in-run author
//! would use (`author::probe`) -- before the round may count it, exactly as
//! a freeze holds it
//! (`HostProbe::prove_can_fail`): it must fail there, or fail there once the
//! inputs it names are moved aside. A failing check needs no proof here: it
//! already blocks the round.
//!
//! - proven: recorded per (base, check id, check text) under
//!   `v2/acceptance/failability/`, so no later round or resume runs it
//!   again, and a re-authored check is never answered by its old text;
//! - it cannot fail: a contract defect no task can fix. It goes back to its
//!   author through the same in-round repair as a crashed check
//!   (`repair::repair_ran`), whose republish gate proves the repair on the
//!   same base; a repair that fails leaves the check a blocking defect;
//! - the host could not run it there, or the run has no base at all: the
//!   round's operational error, naming the check. The round never passes
//!   on it; a round that repeats without progress pauses the run (Issue
//!   262), it never fails it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use archon_workflow::WorkflowResult;
use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::{AcceptanceContract, JudgeDecision, content_digest};
use archon_workflow::v2::acceptance_stage::{ACCEPTANCE_RECORDS_DIR, AcceptanceRoundRecordV1};
use serde::{Deserialize, Serialize};

use super::repair::{Defects, RanDefects, Round, repair_ran};
use crate::command::workflow_task_set::executability::{
    Baseline, HOST_UNPROVEN, HostProbe, executed_text,
};

/// The trigger a cannot-fail repair records.
pub(super) const REPAIR_TRIGGER_CANNOT_FAIL: &str = "cannot_fail";
const SCHEMA: u32 = 1;

#[derive(Default, Serialize, Deserialize)]
struct Proofs {
    schema: u32,
    base: String,
    /// Keyed by [`proof_key`]; `finding` absent means proven.
    checks: BTreeMap<String, Proof>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Proof {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finding: Option<String>,
}

/// A pass the round counts: exactly `check_record`'s rule.
fn passed(result: &CheckResult) -> bool {
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    result.operational_error.is_none()
        && result.exit_code == Some(0)
        && !archon_workflow::acceptance::output_reports_zero_work(&stdout, &stderr)
}

/// The check's id and the exact text the host runs for it.
fn proof_key(contract: &AcceptanceContract, id: &str) -> Option<String> {
    let entry = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id && entry.judgment.verdict == JudgeDecision::Accepted)?;
    let (_, text) = executed_text(entry)?;
    Some(content_digest(format!("{id}\0{text}").as_bytes()))
}

/// The tree before any implementation, as the in-run author finds it
/// (`author::probe`): the run's base commit, else the commit the task set's
/// decomposition recorded, else the checkout's HEAD.
fn base_of(round: &Round<'_>) -> Option<String> {
    let context = round.context;
    (round.base.map(str::to_string)).or_else(|| {
        Baseline::for_task_set(&context.repository, &context.task_root).map(|b| b.commit)
    })
}

fn proofs_path(run_dir: &Path, base: &str) -> PathBuf {
    let sha: String = base
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(40)
        .collect();
    run_dir
        .join(ACCEPTANCE_RECORDS_DIR)
        .join("failability")
        .join(format!("{sha}.json"))
}

/// The proofs recorded for `base`; anything unreadable proves nothing.
fn load(path: &Path, base: &str) -> Proofs {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Proofs>(&bytes).ok())
        .filter(|proofs| proofs.schema == SCHEMA && proofs.base == base)
        .unwrap_or_else(|| Proofs {
            schema: SCHEMA,
            base: base.to_string(),
            checks: BTreeMap::new(),
        })
}

/// Written whole (temporary file, then rename); a write that fails only
/// costs a later round the probe again.
fn save(path: &Path, proofs: &Proofs) {
    let Some(parent) = path.parent() else { return };
    let Ok(bytes) = serde_json::to_vec_pretty(proofs) else {
        return;
    };
    let temporary = path.with_extension("json.tmp");
    if std::fs::create_dir_all(parent).is_ok() && std::fs::write(&temporary, bytes).is_ok() {
        let _ = std::fs::rename(&temporary, path);
    }
}

fn unproven(id: &str, why: &str) -> String {
    format!(
        "{HOST_UNPROVEN} that acceptance check '{id}' can fail: it passed this round, but {why}; a check never shown failing proves nothing, so the round does not count its pass"
    )
}

/// Hold every check that passed this round (and is no defect already) to
/// the base (see the module docs). Checks that remain contract defects join
/// `defects`; `results`, `contract` and `chain_digest` follow any repair.
pub(super) async fn hold_passing(
    round: &Round<'_>,
    run_dir: &Path,
    contract: &mut AcceptanceContract,
    chain_digest: &mut String,
    results: &mut BTreeMap<String, CheckResult>,
    defects: &mut Defects,
    record: &mut AcceptanceRoundRecordV1,
) -> WorkflowResult<()> {
    let passing: BTreeMap<String, String> = (results.iter())
        .filter(|(id, result)| !defects.contains_key(*id) && passed(result))
        .filter_map(|(id, _)| Some((id.clone(), proof_key(contract, id)?)))
        .collect();
    if passing.is_empty() {
        return Ok(());
    }
    let Some(base) = base_of(round) else {
        for id in passing.keys() {
            let why = "the run recorded no pre-implementation tree (no base commit, and no checkout the task set records) to run it on";
            record.operational_errors.push(unproven(id, why));
        }
        return Ok(());
    };
    let path = proofs_path(run_dir, &base);
    let mut proofs = load(&path, &base);
    let mut findings = BTreeMap::new();
    let mut probe_ids = BTreeSet::new();
    for (id, key) in &passing {
        match proofs.checks.get(key).map(|proof| proof.finding.clone()) {
            Some(None) => {}
            Some(Some(finding)) => drop(findings.insert(id.clone(), finding)),
            None => drop(probe_ids.insert(id.clone())),
        }
    }
    if !probe_ids.is_empty() {
        let context = round.context;
        let probe = HostProbe::at(
            context.project.clone(),
            context.repository.clone(),
            context.binding.clone(),
        )
        .with_baseline(Baseline {
            commit: base.clone(),
            repository: context.repository.clone(),
        });
        let proved = probe.prove_can_fail(contract, &probe_ids).await;
        for id in &proved.proven {
            let proof = Proof {
                id: id.clone(),
                finding: None,
            };
            proofs.checks.insert(passing[id].clone(), proof);
        }
        for (id, finding) in &proved.findings {
            let proof = Proof {
                id: id.clone(),
                finding: Some(finding.clone()),
            };
            proofs.checks.insert(passing[id].clone(), proof);
            findings.insert(id.clone(), finding.clone());
        }
        for (id, why) in &proved.unproven {
            record.operational_errors.push(unproven(id, why));
        }
        save(&path, &proofs);
    }
    if findings.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = findings.keys().cloned().collect();
    let ran = RanDefects {
        findings,
        trigger: REPAIR_TRIGGER_CANNOT_FAIL,
        evidence: "cannot-fail",
        why: "No task can fix a check that passes whether or not its criterion holds",
    };
    // The repair is proven on the same tree its original failed to fail on.
    let round = Round {
        base: Some(&base),
        ..*round
    };
    let left = repair_ran(&round, contract, chain_digest, results, record, ran).await?;
    // A repair is published only once its republish gate proved it on this
    // same base: its new text is proven.
    let repaired = (record.contract_repairs.last())
        .is_some_and(|repair| repair.repaired && repair.trigger == REPAIR_TRIGGER_CANNOT_FAIL);
    if repaired {
        for id in ids.iter().filter(|id| !left.contains_key(*id)) {
            if let Some(key) = proof_key(contract, id) {
                let proof = Proof {
                    id: id.clone(),
                    finding: None,
                };
                proofs.checks.insert(key, proof);
            }
        }
        save(&path, &proofs);
    }
    defects.extend(left);
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_failability_tests.rs"]
mod tests;
