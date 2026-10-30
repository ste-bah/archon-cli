//! Bounded re-author and re-judge of named acceptance checks.
//!
//! This loop asks a read-only author to repair refuted checks, re-judges
//! replies, and stops after [`REAUTHOR_ATTEMPTS`] idle attempts. Unnamed
//! entries retain their original judgment. It follows the decomposition
//! acceptance shapes and grounding rules.

use std::collections::BTreeMap;

use archon_core::agents::harness::HOST_READ_ONLY_TOOLS;
#[cfg(test)]
use archon_workflow::task_set_contract::AcceptanceCheck;
use archon_workflow::task_set_contract::{
    AcceptanceCriterion, JudgeDecision, refuted_check_message,
};

use super::executability::ExecutabilityProbe;
use super::*;
#[path = "workflow_acceptance_reauthor_reporting.rs"]
mod reporting;
pub(crate) use reporting::{not_accepted_lines, not_accepted_report};
#[path = "workflow_acceptance_reauthor_scope.rs"]
mod scope;
pub(crate) use scope::{AuthorScope, site_uses_build_cache};
#[path = "workflow_acceptance_reauthor_parse.rs"]
mod reply_parse;
#[cfg(test)]
use reply_parse::candidate_entry;
use reply_parse::candidate_entry_with_markers;
#[path = "workflow_acceptance_reauthor_check_defects.rs"]
mod check_defects;
#[path = "workflow_acceptance_reauthor_passability.rs"]
mod passability_pending;

#[path = "workflow_acceptance_reauthor_cache.rs"]
pub(crate) mod cache;

/// Consecutive author-then-judge attempts that repair no named check.
pub(crate) const REAUTHOR_ATTEMPTS: usize = 3;

/// The exact-tool marker the subagent adapters honor (`archon-pipeline`
/// `subagent_adapter.rs`, `workflow_live_v2_client.rs`): the author reads the
/// repository and the PRD and writes nothing. The tools are the host agent's
/// own definition (`archon_core::agents::harness`), so the call's allowlist
/// and the resolved agent cannot drift apart.
const EXACT_TOOL_POLICY_MARKER: &str = "__ARCHON_EXACT_TOOLS__";
const AUTHOR_TOOLS: [&str; 3] = HOST_READ_ONLY_TOOLS;

/// The two entry shapes the decomposition author is shown.
const ENTRY_SHAPES: &str = r#"[{"id":"<exact acceptance id defined by the PRD>","criterion":"","check":{"kind":"floor","contract":{"kind":"<deliverable kind>","artifact_path":"<repository-relative artifact path>","artifact_format":"json","required_true_fields":["<field that must be true>"],"typed_verifier_command":"<command that exercises the deliverable and fails when the criterion is false>"}},"gap_permitted":false,"covers":["<requirement id defined by the PRD whose violation makes this check fail>"],"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}},{"id":"<exact acceptance id defined by the PRD>","criterion":"","check":{"kind":"command","command_block":true,"cwd":"project_root"},"gap_permitted":false,"covers":["<requirement id defined by the PRD whose violation makes this check fail>"],"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}]"#;

fn entry<'a>(contract: &'a AcceptanceContract, id: &str) -> Option<&'a AcceptanceCriterion> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id)
}

fn entry_mut<'a>(
    contract: &'a mut AcceptanceContract,
    id: &str,
) -> Option<&'a mut AcceptanceCriterion> {
    contract
        .acceptance
        .iter_mut()
        .chain(&mut contract.supplementary)
        .find(|entry| entry.id == id)
}

/// What, beyond the judge, a re-authored check must clear before it is
/// accepted: the executability probe, and any finding the caller already
/// holds per id (the crash an acceptance round observed).
pub(crate) struct ReauthorGate<'a> {
    pub(crate) probe: &'a dyn ExecutabilityProbe,
    pub(crate) seeds: &'a BTreeMap<String, String>,
}

/// Accepted checks and per-check findings that remain pending, from either
/// an authored repair or a host proof.
#[derive(Debug, Clone)]
pub(crate) struct ReauthorProgress {
    pub(crate) contract: AcceptanceContract,
    pub(crate) unproven: BTreeMap<String, String>,
    pub(crate) host_faults: BTreeMap<String, crate::command::workflow_host_fault::Class>,
    /// Checks still waiting on another authored reply or judge attempt.
    pub(crate) authored_pending: BTreeMap<String, String>,
    pub(crate) attempts: usize,
}

/// Re-author and re-judge exactly `ids` in `contract`, bounded. Returns the
/// contract with those entries replaced by accepted ones that did not crash
/// in their own code when the host ran them, every other entry untouched,
/// and findings for named checks still unresolved after the bounded loop.
pub(crate) async fn reauthor(
    client: &dyn WorkflowLlmClient,
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
    scope: &AuthorScope,
    judge_model: &str,
    gate: &ReauthorGate<'_>,
) -> Result<AcceptanceContract> {
    reauthor_with_cache(client, contract, ids, scope, judge_model, gate, None).await
}

pub(crate) async fn reauthor_with_cache(
    client: &dyn WorkflowLlmClient,
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
    scope: &AuthorScope,
    judge_model: &str,
    gate: &ReauthorGate<'_>,
    state_dir: Option<&Path>,
) -> Result<AcceptanceContract> {
    let progress =
        reauthor_partial_with_cache(client, contract, ids, scope, judge_model, gate, state_dir)
            .await?;
    let authored_pending: BTreeSet<_> = progress.authored_pending.keys().cloned().collect();
    if !authored_pending.is_empty() {
        let lines = authored_pending
            .iter()
            .map(|id| progress.unproven.get(id).cloned().unwrap_or_default())
            .collect::<Vec<_>>();
        let escalation =
            escalation::baseline_escalation(&authored_pending, &lines, progress.attempts)
                .unwrap_or_default();
        return Err(ReauthorStalled {
            report: format!("{}\nthe re-author stopped after {} attempt(s): the last {REAUTHOR_ATTEMPTS} repaired no named check{escalation}", reporting::report(&lines, progress.attempts), progress.attempts),
            pending: authored_pending.into_iter().collect(),
        }.into());
    }
    if !progress.unproven.is_empty() {
        return Err(super::executability::HostUnproven(progress.unproven).into());
    }
    Ok(progress.contract)
}

pub(crate) async fn reauthor_partial_with_cache(
    client: &dyn WorkflowLlmClient,
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
    scope: &AuthorScope,
    judge_model: &str,
    gate: &ReauthorGate<'_>,
    state_dir: Option<&Path>,
) -> Result<ReauthorProgress> {
    let mut feedback: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unseeded_accepted = BTreeSet::new();
    for id in ids {
        let frozen = entry(contract, id)
            .ok_or_else(|| anyhow!("check '{id}' is not in the acceptance contract"))?;
        if frozen.judgment.verdict == JudgeDecision::Accepted && !gate.seeds.contains_key(id) {
            unseeded_accepted.insert(id.clone());
        }
    }
    // An accepted check named for re-authoring is run first, so an author
    // repairing a crash is shown the crash itself.
    // A5: how each runs as frozen is also the verdict its repair is held to.
    let crashed = if unseeded_accepted.is_empty() {
        BTreeMap::new()
    } else {
        gate.probe
            .hold_originals(contract, &unseeded_accepted)
            .await
    };
    // A frozen check the host could not run is being replaced anyway; it
    // simply holds its repair to nothing.
    let _ = gate.probe.take_unproven();
    // How the frozen checks failed on the baseline is not their repairs'.
    let _ = gate.probe.take_baseline_runs();
    // Issue 275: a replacement is held to the requirements it covers.
    let prd_text = std::fs::read_to_string(&scope.prd_path)
        .with_context(|| format!("reading the PRD {}", scope.prd_path.display()))?;
    let reply_leaked_markers =
        crate::command::workflow_live::load_reply_leaked_markers_for_fixed(&scope.project_root)?;
    for id in ids {
        let frozen = entry(contract, id).expect("named ids were checked above");
        let first = if let Some(seed) = gate.seeds.get(id).or_else(|| crashed.get(id)) {
            seed.clone()
        } else if frozen.judgment.verdict == JudgeDecision::Accepted {
            format!(
                "check '{id}' was named for re-authoring; replace it with a check that fails whenever the criterion is false"
            )
        } else {
            refuted_check_message(id, &frozen.judgment.reason, &frozen.judgment.counterexample)
        };
        feedback.insert(id.clone(), vec![first]);
    }
    let mut working = contract.clone();
    let mut pending = ids.clone();
    let mut reused = BTreeMap::new();
    let mut unproven_checks = BTreeMap::new();
    let mut host_faults = BTreeMap::new();
    let mut judge_retry_candidates = passability_pending::JudgeRetryCandidates::default();
    let refused = author::refused_forms(scope, gate.probe);
    let keyed_judge = client.resolve_model_alias(judge_model); // the model that judges
    if let Some(state_dir) = state_dir {
        for id in ids {
            let frozen = entry(contract, id).expect("named ids were checked above");
            match cache::load(
                state_dir,
                frozen,
                scope,
                &keyed_judge,
                client.provider_id().as_deref(),
                gate.probe,
            ) {
                // A saved candidate is held to every rule a fresh reply is.
                Ok(Some(candidate)) => match author::live_root_refusal(candidate, &refused)
                    .and_then(|candidate| check_defects::check_defects(&working, candidate, scope))
                {
                    Ok(candidate) => {
                        *entry_mut(&mut working, id).expect("named id") = candidate.clone();
                        reused.insert(id.clone(), candidate);
                        eprintln!("acceptance re-author reused check {id}");
                    }
                    Err(reason) => feedback.get_mut(id).expect("feedback per id").push(reason),
                },
                Ok(None) => {}
                Err(reason) => eprintln!("acceptance re-author ignored saved check {id}: {reason}"),
            }
        }
    }
    // Bounded by progress: every attempt that repairs a check resets the
    // count, and with finitely many named checks the loop always ends.
    let (mut attempt, mut idle) = (0, 0);
    let mut prefetch_cache =
        crate::command::workflow_task_set::judge::ReauthorPrefetchCache::default();
    while (!pending.is_empty() || !reused.is_empty()) && idle < REAUTHOR_ATTEMPTS {
        attempt += 1;
        let before = pending.len();
        let mut authored = Vec::new();
        for id in &pending {
            if reused.contains_key(id) {
                continue;
            }
            if let Some(candidate) = judge_retry_candidates.take(id) {
                authored.push(candidate);
                continue;
            }
            let frozen = entry(contract, id).expect("named ids were checked above");
            let notes = feedback.get_mut(id).expect("feedback seeded per id");
            // Findings evolve after every rejected draft. Snapshot the named
            // inputs from the current notes immediately before each prompt.
            let prefetched_files =
                author::prefetch_files_cached(scope, frozen, notes, &mut prefetch_cache);
            let reply = author::author_entry_with_prefetch(
                client,
                scope,
                frozen,
                notes,
                attempt,
                &prefetched_files,
            )
            .await?;
            match candidate_entry_with_markers(&reply, frozen, &reply_leaked_markers)
                .and_then(|candidate| author::live_root_refusal(candidate, &refused))
                .and_then(|candidate| check_defects::check_defects(&working, candidate, scope))
            {
                Ok(candidate) => authored.push(candidate),
                Err(reason) => notes.push(reason),
            }
        }
        let mut accepted = BTreeSet::new();
        for (id, candidate) in &reused {
            *entry_mut(&mut working, id).expect("named id") = candidate.clone();
            accepted.insert(id.clone());
        }
        if !authored.is_empty() {
            let mut subset = contract.clone();
            let acceptance_ids: BTreeSet<_> =
                contract.acceptance.iter().map(|e| e.id.clone()).collect();
            let (acceptance, supplementary): (Vec<_>, Vec<_>) = authored
                .into_iter()
                .partition(|candidate| acceptance_ids.contains(&candidate.id));
            subset.acceptance = acceptance;
            subset.supplementary = supplementary;
            let retry_contract = subset.clone();
            let judged = judge::judge_entries_with_roots(
                client,
                subset,
                judge_model,
                &judge::JudgeRoots {
                    project: Some(scope.project_root.clone()),
                    repository: Some(scope.repository_root.clone()),
                    task_root: Some(scope.task_root.clone()),
                    repository_commit: scope
                        .baseline
                        .as_ref()
                        .map(|baseline| baseline.commit.clone()),
                },
            )
            .await;
            // D2: a judge that could not complete holds the candidates of
            // its call, by whose fault it is; every proven sibling goes on.
            let judged = match judged {
                Ok(judged) => Some(judged),
                Err(error) => Some(passability_pending::hold_judge_failure(
                    error,
                    &retry_contract,
                    &mut judge_retry_candidates,
                    &mut feedback,
                    (&mut pending, &mut reused, &mut unproven_checks),
                    &mut host_faults,
                )?),
            };
            if let Some(judged) = judged {
                for candidate in judged.acceptance.into_iter().chain(judged.supplementary) {
                    if candidate.judgment.verdict == JudgeDecision::Accepted {
                        accepted.insert(candidate.id.clone());
                        let id = candidate.id.clone();
                        *entry_mut(&mut working, &id).expect("named id") = candidate;
                    } else if let Some(notes) = feedback.get_mut(&candidate.id) {
                        notes.push(refuted_check_message(
                            &candidate.id,
                            &candidate.judgment.reason,
                            &candidate.judgment.counterexample,
                        ));
                    }
                }
            }
            // Judged accepted is not yet publishable: a check that crashes in
            // its own code can never assert its criterion. It goes back to
            // its author with the crash, and the frozen entry is restored.
        }
        if !accepted.is_empty() {
            let _ = gate.probe.take_baseline_runs();
            let mut crashed = gate.probe.script_defects(&working, &accepted).await;
            let unproven: BTreeMap<String, _> = (gate.probe.take_unproven_causes().into_iter())
                .filter(|(id, _)| accepted.contains(id))
                .collect();
            // F6: only a program neither the site nor the host has is the
            // author's; a site lacking what the host has stays the host's.
            // Issue 548 replaces that second half: no host repair ever makes
            // a site provide a program it lacks, so that check is its
            // author's too, in the same budget and with no strike: no retry
            // can prove it.
            for (id, (reason, cause)) in unproven {
                if matches!(
                    cause,
                    super::executability::UnprovenCause::ProgramAbsent
                        | super::executability::UnprovenCause::SiteLacks
                ) {
                    crashed.insert(
                        id.clone(),
                        format!("the check invokes a program unavailable at the probe site ({reason}); choose a check using what the site provides"),
                    );
                    continue;
                }
                *entry_mut(&mut working, &id).expect("named id") =
                    entry(contract, &id).expect("named id").clone();
                pending.remove(&id);
                reused.remove(&id);
                host_faults.insert(id.clone(), cause.class());
                unproven_checks.insert(id, reason);
            }
            // Issue 275: a replacement that failed on the baseline must also
            // be able to pass: one whose failure is its own setup breaking a
            // rule a correct implementation keeps goes back with that output.
            let sound: BTreeSet<String> = (accepted.iter())
                .filter(|id| !crashed.contains_key(*id))
                .cloned()
                .collect();
            let mut passability_pending = BTreeSet::new();
            let cannot_pass = match gate.probe.take_baseline_runs() {
                Some(runs) if !sound.is_empty() => {
                    let audit = cache::passability_audit_dir(state_dir.unwrap_or(&scope.task_root));
                    match super::passability::cannot_pass_findings(
                        client,
                        judge_model,
                        &scope.repository_root,
                        &scope.project_root,
                        &working,
                        &sound,
                        &runs,
                        &prd_text,
                        Some(&audit),
                    )
                    .await
                    {
                        Ok(findings) => findings,
                        // D3: per check, by whose fault it is; siblings go on.
                        Err(error) => {
                            let held = passability_pending::passability_failure(&error, &sound)
                                .ok_or(error)?;
                            for id in held.ids {
                                *entry_mut(&mut working, &id).expect("named id") =
                                    entry(contract, &id).expect("named id").clone();
                                if held.authored {
                                    passability_pending.insert(id.clone());
                                    let notes = feedback.get_mut(&id).expect("feedback per id");
                                    notes.push(held.finding.clone());
                                } else {
                                    pending.remove(&id);
                                    reused.remove(&id);
                                    host_faults.insert(id.clone(), held.fault_class);
                                    unproven_checks.insert(id, held.finding.clone());
                                }
                            }
                            BTreeMap::new()
                        }
                    }
                }
                _ => BTreeMap::new(),
            };
            for id in accepted {
                if unproven_checks.contains_key(&id) {
                    continue;
                }
                if passability_pending.contains(&id) {
                    continue;
                }
                if let Some(finding) = crashed.get(&id).or_else(|| cannot_pass.get(&id)) {
                    // A4/A5: a check that cannot be shown able to fail (or
                    // would weaken its original) goes back like a crash.
                    *entry_mut(&mut working, &id).expect("named id") =
                        entry(contract, &id).expect("named id").clone();
                    reused.remove(&id);
                    feedback
                        .get_mut(&id)
                        .expect("feedback seeded per id")
                        .push(finding.clone());
                } else {
                    pending.remove(&id);
                    reused.remove(&id);
                    if let Some(state_dir) = state_dir {
                        let candidate = entry(&working, &id).expect("accepted named id");
                        cache::store(
                            state_dir,
                            candidate,
                            contract,
                            scope,
                            &keyed_judge,
                            client.provider_id().as_deref(),
                            gate.probe,
                        )?;
                    }
                }
            }
        }
        idle = if pending.len() < before { 0 } else { idle + 1 };
    }
    // Exhausting authored repair attempts is a per-check pending result.
    // Keep the latest finding with each original entry so a caller can
    // publish proven siblings and retry these checks on resume.
    let mut authored_pending = BTreeMap::new();
    for id in &pending {
        let finding = feedback
            .get(id)
            .and_then(|notes| notes.last())
            .cloned()
            .unwrap_or_else(|| format!("check '{id}' remains unaccepted after re-author attempts"));
        unproven_checks
            .entry(id.clone())
            .or_insert_with(|| finding.clone());
        authored_pending.insert(id.clone(), finding);
    }
    Ok(ReauthorProgress {
        contract: working,
        unproven: unproven_checks,
        host_faults,
        authored_pending,
        attempts: attempt,
    })
}

#[path = "workflow_acceptance_reauthor_author.rs"]
pub(crate) mod author;
#[path = "workflow_acceptance_reauthor_escalation.rs"]
mod escalation;
#[path = "workflow_acceptance_reauthor_stalled.rs"]
mod stalled;
pub(crate) use stalled::ReauthorStalled;

#[cfg(test)]
#[path = "workflow_acceptance_reauthor_boundary_tests.rs"]
mod boundary_tests;
#[cfg(test)]
#[path = "workflow_acceptance_reauthor_live_root_tests.rs"]
mod live_root_tests;
#[cfg(test)]
#[path = "workflow_acceptance_reauthor_prefetch_tests.rs"]
mod prefetch_tests;
#[cfg(test)]
#[path = "workflow_acceptance_reauthor_prompt_tests.rs"]
mod prompt_tests;
#[cfg(test)]
#[path = "workflow_acceptance_reauthor_test_client.rs"]
pub(crate) mod test_client;
#[cfg(test)]
#[path = "workflow_acceptance_reauthor_tests.rs"]
mod tests;
