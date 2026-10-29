//! Running the input mutation and reading its verdict (A4; the mutation
//! itself is `workflow_acceptance_executability_mutation`).
//!
//! Each mutated check runs in its own fresh copy (or scratch observation),
//! never a copy another mutation changed. Its run proves the check can fail
//! only when it moved at least one input, restored every one (the epilogue's
//! marker), and then FAILED ON ITS OWN TERMS: a run that crashed in the
//! check's own code (`crash_findings`) or could not start a program
//! (exit 126/127) failed for no reason its criterion gives. What the host
//! could not complete -- an operational error, a killed run, an input left
//! unrestored, the live-root guard -- is UNPROVEN, the host's; never the
//! author's.

use std::path::{Path, PathBuf};

#[path = "workflow_acceptance_executability_prove_plan.rs"]
mod plan;
#[cfg(test)]
pub(super) fn mutation_plan(
    names: &[String],
    discovery: &mutation::InputDiscovery,
    entry: &AcceptanceCriterion,
    markers: &mutation::Markers,
) -> archon_workflow::acceptance_scratch::MutationPlan {
    plan::plan(names, discovery, entry, markers).plan
}

#[path = "workflow_acceptance_executability_prove_diagnostics.rs"]
mod diagnostics;
pub(super) use diagnostics::{
    baseline_pass_finding, baseline_pass_finding_without_discovered_input, describe_candidates,
    describe_candidates_with_run_state,
};

#[path = "workflow_acceptance_executability_prove_helpers.rs"]
mod helpers;
pub(super) use helpers::author_recreated_finding;
#[cfg(test)]
pub(super) use helpers::operational_error_diagnostics;
pub(super) use helpers::unattributed_recreation_finding;
pub(super) use helpers::{
    MutationFinding, all_moved_inputs_restored, describe_states, describe_status, input_states,
    mutation_host_or_author_finding, recreated_inputs, recreated_paths,
};

use super::*;

/// Findings for every check of `passing` (which passed on `baseline`) that
/// cannot be shown to fail with the data it names moved aside; each one
/// that fails then is a regression guard, recorded as a diagnostic.
pub(super) async fn prove(
    probe: &HostProbe,
    baseline: &Baseline,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    passing: &[String],
    copy_layouts: &BTreeMap<String, super::hermetic::CopyLayout>,
) -> BTreeMap<String, String> {
    let short: String = baseline.commit.chars().take(12).collect();
    let mut findings = BTreeMap::new();
    let live = [
        baseline
            .repository
            .canonicalize()
            .map(archon_shell::paths::plain),
        probe.project.canonicalize().map(archon_shell::paths::plain),
    ];
    let live: Vec<&Path> = live.iter().flatten().map(PathBuf::as_path).collect();
    for id in passing {
        let entry = (contract.acceptance.iter())
            .chain(&contract.supplementary)
            .find(|entry| &entry.id == id);
        let bound_accepted = entry.is_some_and(|entry| {
            entry.judgment.verdict == JudgeDecision::Accepted
                && refs.iter().any(|reference| {
                    reference.acceptance_id == *id
                        && reference.chain_digest == digest
                        && executed_text(entry).is_some_and(|(_, text)| {
                            reference.command_digest == content_digest(text.as_bytes())
                        })
                })
        });
        let discovery = (contract.acceptance.iter())
            .chain(&contract.supplementary)
            .find(|entry| &entry.id == id)
            .map(|entry| {
                mutation::input_discovery_for_check(
                    entry,
                    &probe.project,
                    &baseline.repository,
                    copy_layouts.get(id),
                )
            })
            .unwrap_or_default();
        let names = discovery.eligible.clone();
        let baseline_finding = format!(
            "{}; considered inputs: {}",
            baseline_pass_finding(id, &short),
            describe_candidates(&discovery.candidates)
        );
        if names.is_empty() {
            // Unresolvable inputs never count as moved; a check with no
            // resolvable input routes here so its author can make paths explicit.
            let finding = if discovery.candidates.is_empty() {
                format!(
                    "{}; considered inputs: {}; no movable input exists under the selected root",
                    baseline_pass_finding_without_discovered_input(id, &short),
                    describe_candidates(&discovery.candidates)
                )
            } else {
                format!("{baseline_finding}; no movable input exists under the selected root")
            };
            findings.insert(id.clone(), finding);
            continue;
        }
        // One fresh copy per mutated check; a retry meets the same mutation.
        #[cfg(test)]
        let injected = probe.take_prove_run();
        #[cfg(not(test))]
        let injected: Option<(super::repairs::TreeRun, mutation::Markers)> = None;
        let markers = injected
            .as_ref()
            .map(|(_, markers)| markers.clone())
            .unwrap_or_else(|| probe.mutation_markers(baseline, contract, id));
        let entry = entry.expect("input discovery came from an acceptance entry");
        let built_plan = plan::plan(&names, &discovery, entry, &markers);
        debug_assert!(discovery.eligible.iter().all(|name| {
            built_plan
                .plan
                .inputs
                .iter()
                .any(|input| &input.named_path == name)
                || built_plan.covered_inputs.contains_key(name)
        }));
        let move_plan = built_plan.plan;
        // Marker indexes describe the plan, so derive every indexed list from
        // its inputs after sorting ancestors before their children.
        let names: Vec<String> = move_plan
            .inputs
            .iter()
            .map(|input| input.named_path.clone())
            .collect();
        let inputs = BTreeMap::from([(id.clone(), names.clone())]);
        let move_paths = BTreeMap::from([(
            id.clone(),
            names
                .iter()
                .map(|name| {
                    discovery
                        .candidates
                        .iter()
                        .find(|candidate| &candidate.path == name)
                        .and_then(|candidate| candidate.move_path.clone())
                        .unwrap_or_else(|| name.clone())
                })
                .collect(),
        )]);
        let move_roots = BTreeMap::from([(
            id.clone(),
            names
                .iter()
                .map(|name| {
                    discovery
                        .candidates
                        .iter()
                        .find(|candidate| &candidate.path == name)
                        .and_then(|candidate| candidate.move_root.clone())
                })
                .collect(),
        )]);
        let mutation_plans = BTreeMap::from([(id.clone(), move_plan)]);
        let mutated =
            mutation::mutated(contract, &inputs, &move_paths, &move_roots, &live, &markers);
        let save_predicate_markers = markers.clone();
        let save_predicate_names = names.clone();
        let save_predicates = BTreeMap::from([(
            id.clone(),
            Arc::new(
                move |result: &archon_workflow::acceptance_scratch::CheckResult| {
                    all_moved_inputs_restored(
                        result,
                        &save_predicate_markers,
                        &save_predicate_names,
                    )
                },
            ) as super::resume::SavePredicate,
        )]);
        let Ok(digest) = super::contract_digest(&mutated) else {
            probe.unproven(id, "the mutated contract could not be encoded".into());
            continue;
        };
        let ids = BTreeSet::from([id.clone()]);
        let refs = super::refs_for(&mutated, &digest, &ids);
        let run = match injected {
            Some((run, _)) => run,
            None => {
                super::repairs::tree_results_with_save_predicates(
                    probe,
                    baseline,
                    &mutated,
                    &digest,
                    &refs,
                    None,
                    false,
                    &save_predicates,
                    &mutation_plans,
                )
                .await
            }
        };
        let result = run.results.get(id);
        if result.is_none() {
            probe.unproven_as(
                id,
                format!("it passed on the pre-implementation tree at {short}, but the run produced no result, so no input states or exit status were recorded: {}", run.unrun(id).unwrap_or_default()),
                UnprovenCause::from_class(run.fault_class),
            );
            continue;
        }
        if result.is_some_and(|result| super::repairs::timed_out(&result.result)) {
            let result = &result.expect("a timed out result").result;
            let states = input_states(result, &markers, &names);
            let diagnostics = format!(
                "{}; status/signal: {}",
                describe_states(&states),
                describe_status(markers.status(result))
            );
            if !all_moved_inputs_restored(result, &markers, &names) {
                super::sites::discard_mutation_evidence(probe, baseline, &mutated, id);
            }
            if let Some(finding) = super::silent::settle_timed_out_with_diagnostics(
                probe,
                &baseline.commit,
                &mutated,
                id,
                &diagnostics,
            ) {
                findings.insert(id.clone(), finding);
            }
            continue;
        }
        let setup_failure = match result {
            Some(result) if result.operational_error.is_none() => {
                let at =
                    super::silent::baseline_context(probe, &mutated, result.copy_layout.as_ref());
                match super::silent::setup_path_failure_off_thread(&mutated, result, &at).await {
                    Ok(finding) => finding,
                    Err(error) => {
                        probe.unproven(
                            id,
                            format!("the mutation run completed, but the host could not decide whether its setup path was missing ({error})"),
                        );
                        continue;
                    }
                }
            }
            _ => None,
        };
        if result.is_some_and(|result| !all_moved_inputs_restored(result, &markers, &names)) {
            // The result itself cannot be reused if its disposable copy did
            // not finish restoring the inputs it moved.
            super::sites::discard_mutation_evidence(probe, baseline, &mutated, id);
        }
        let result = result.expect("no-result case handled above");
        let generated = markers.generated(result, &names);
        let recreated = recreated_inputs(result, &markers, &names);
        let moved: BTreeSet<String> = markers.moved(result, &names).into_iter().collect();
        let original_recreated: BTreeSet<String> =
            recreated_paths(&input_states(&result.result, &markers, &names))
                .into_iter()
                .collect();
        let baseline_finding = format!(
            "{}; considered inputs: {}",
            baseline_pass_finding(id, &short),
            describe_candidates_with_run_state(
                &discovery.candidates,
                &generated,
                &recreated,
                &moved,
                &original_recreated,
                &markers.move_states(&result.result, &names),
                &built_plan.covered_inputs,
                true,
            )
        );
        match mutation_host_or_author_finding(
            result,
            &markers,
            &names,
            setup_failure,
            &baseline_finding,
        ) {
            Some(MutationFinding::HostUnproven(reason)) => {
                // What the check's own run caused is its author's (D1).
                let unproven = format!(
                    "it passed on the pre-implementation tree at {short}, and the host could not run it with the data it names moved aside ({reason}), so it is not proven able to fail"
                );
                let fault = super::check_fault::of_mutation(&result.result, &markers);
                let at = format!(
                    "on the pre-implementation tree at {short} with the data it names moved aside"
                );
                let key = (Some(baseline.commit.as_str()), &mutated, id.as_str());
                if let Some(finding) = super::check_fault::settle(probe, key, fault, &at, unproven)
                {
                    findings.insert(id.clone(), finding);
                }
                continue;
            }
            Some(MutationFinding::AuthorFinding(finding)) => {
                findings.insert(id.clone(), finding);
                continue;
            }
            None => {}
        }
        let at = super::silent::baseline_context(probe, &mutated, result.copy_layout.as_ref());
        super::silent::ran(probe, &baseline.commit, &mutated, id);
        // It gave a verdict with its data moved aside: an earlier strike of
        // a signal there is forgotten.
        super::silent::clear(probe, &baseline.commit, &mutated, id);
        let moved_all = markers.moved(result, &names);
        let moved: Vec<String> = moved_all.iter().cloned().collect();
        // Issue 328: nor did a run that failed for its host (a program that
        // could not start, a tree that did not build) fail on its own terms.
        // The executor classified the generated mutation wrapper, whose
        // shell text is not the author's original inline program. Reclassify
        // the captured stderr against the original command before deciding
        // whether a failed mutation was an assertion or the check's own crash.
        let mut raw_mutation_result = result.result.clone();
        raw_mutation_result.classification = None;
        let protocol = [
            markers.moved_prefix(),
            markers.move_state_prefix(),
            markers.generated_prefix(),
            markers.recreated_prefix(),
            format!("archon-mutation-check-recreated-{}: ", markers.nonce()),
            format!("archon-mutation-guard-{}:", markers.nonce()),
        ];
        raw_mutation_result.stderr = String::from_utf8_lossy(&raw_mutation_result.stderr)
            .lines()
            .filter(|line| !protocol.iter().any(|prefix| line.starts_with(prefix)))
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        let crashed = !super::crash_findings(contract, [&raw_mutation_result]).is_empty();
        let silent = if crashed {
            None
        } else {
            super::silent::silent_failure_off_thread(&mutated, result, &at).await
        };
        if let Some((why, cause @ UnprovenCause::TransientHost)) = &silent {
            probe.unproven_as(
                id,
                format!("the host could not decide the mutation proof: {why}"),
                *cause,
            );
            continue;
        }
        let crashed = crashed || silent.is_some();
        if moved.is_empty() {
            findings.insert(
                id.clone(),
                format!("{baseline_finding}; no movable input exists under the selected root"),
            );
        } else if super::baseline::passed(result) {
            if !recreated.is_empty() {
                findings.insert(
                    id.clone(),
                    author_recreated_finding(&baseline_finding, &recreated),
                );
            } else if !original_recreated.is_empty() {
                findings.insert(
                    id.clone(),
                    unattributed_recreation_finding(&baseline_finding),
                );
            } else if bound_accepted && !crashed {
                let move_states = markers.move_states(&result.result, &names);
                let output = output_evidence(&result.result);
                let basis = format!(
                    "accepted on the stored judge verdict because the check passed on the pre-implementation tree at {short}; mutation proof: inputs={move_states:?}; moved={moved_all:?}; mutated_exit_code={:?}; output_excerpt={output:?}",
                    result.result.exit_code
                );
                probe.note_regression_guard(id, basis.clone());
                probe.note(format!(
                    "check '{id}': passed on the pre-implementation tree at {short}; accepted as a regression guard on its stored accepted judge verdict, bound to the current contract and check digests (the moved text-derived paths did not make it fail)"
                ));
            } else {
                findings.insert(
                    id.clone(),
                    format!(
                        "{baseline_finding}; it still passed with {} moved aside, so it does not depend on them",
                        moved.join(", ")
                    ),
                );
            }
        } else if crashed {
            findings.insert(
                id.clone(),
                format!(
                        "{baseline_finding}; with {} moved aside it only crashed, failed to start a program or failed to build, which says nothing about its criterion",
                    moved.join(", ")
                ),
            );
        } else {
            let move_states = markers.move_states(&result.result, &names);
            let output = output_evidence(&result.result);
            let basis = format!(
                "the check passed on the pre-implementation tree at {short} and failed on its own assertion with inputs moved; inputs={move_states:?}; moved={moved_all:?}; mutated_exit_code={:?}; output_excerpt={output:?}",
                result.result.exit_code
            );
            probe.note_regression_guard(id, basis.clone());
            probe.note(format!(
                "check '{id}': its criterion already holds on the pre-implementation tree at {short} (it passed there); kept as a regression guard, proven able to fail: it fails when {} is moved aside; {basis}",
                moved.join(", ")
            ));
        }
    }
    findings
}

fn output_evidence(result: &archon_workflow::acceptance_scratch::CheckResult) -> String {
    // Captured output is already bounded by the capture limit, so the whole
    // capture is redacted before the excerpt keeps its tail: a cut before
    // redaction could split a secret and leave part of it in the record.
    fn bounded_excerpt(bytes: &[u8], budget: usize) -> String {
        let decoded = String::from_utf8_lossy(bytes);
        let redacted = archon_observability::redaction::redact_secret_values(&decoded);
        archon_workflow::failure_evidence::failure_evidence(redacted.as_bytes(), budget)
    }
    let stdout = bounded_excerpt(
        &result.stdout,
        crate::command::workflow_task_set::passability::evidence::STDOUT_BYTES,
    );
    let stderr = bounded_excerpt(
        &result.stderr,
        crate::command::workflow_task_set::passability::evidence::STDERR_BYTES,
    );
    let excerpt = format!("stdout: {stdout}\nstderr: {stderr}");
    excerpt
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_prove_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_prove_alias_tests.rs"]
mod alias_tests;
