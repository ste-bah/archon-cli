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

#[path = "workflow_acceptance_executability_prove_diagnostics.rs"]
mod diagnostics;
pub(super) use diagnostics::{
    baseline_pass_finding, baseline_pass_finding_without_discovered_input, describe_candidates,
    describe_candidates_with_run_state,
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
        let markers = probe.mutation_markers(baseline, contract, id);
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
        let entry = entry.expect("input discovery came from an acceptance entry");
        let move_plan = plan::plan(&names, &discovery, entry, &markers);
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
        let run = super::repairs::tree_results_with_save_predicates(
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
        .await;
        let result = run.results.get(id);
        if result.is_none() {
            probe.unproven(
                id,
                format!(
                    "it passed on the pre-implementation tree at {short}, but the run produced no result, so no input states or exit status were recorded"
                ),
            );
            continue;
        }
        if result.is_some_and(|result| super::repairs::timed_out(&result.result)) {
            // #356: the mutated check (its nonce kept across retries) is
            // judged on the base like any other stall.
            let commit = &baseline.commit;
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
                commit,
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
                super::silent::setup_path_failure_off_thread(&mutated, result, &at).await
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
                probe.unproven(
                    id,
                    format!(
                        "it passed on the pre-implementation tree at {short}, and the host could not run it with the data it names moved aside ({reason}), so it is not proven able to fail"
                    ),
                );
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
        let moved_all = markers.moved(result, &names);
        let moved: Vec<String> = moved_all
            .iter()
            .cloned()
            .into_iter()
            .filter(|path| !generated.contains(path) && !recreated.contains(path))
            .collect();
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
            format!("archon-mutation-guard-{}:", markers.nonce()),
        ];
        raw_mutation_result.stderr = String::from_utf8_lossy(&raw_mutation_result.stderr)
            .lines()
            .filter(|line| !protocol.iter().any(|prefix| line.starts_with(prefix)))
            .collect::<Vec<_>>()
            .join("\n")
            .into_bytes();
        let crashed = !super::crash_findings(contract, [&raw_mutation_result]).is_empty()
            || super::silent::silent_failure_off_thread(&mutated, result, &at)
                .await
                .is_some();
        if moved.is_empty() {
            findings.insert(
                id.clone(),
                format!("{baseline_finding}; no movable input exists under the selected root"),
            );
        } else if super::baseline::passed(result) {
            if bound_accepted && recreated.is_empty() && !crashed {
                let basis = format!(
                    "accepted on the stored judge verdict because the check passed on the pre-implementation tree at {short} and the moved text-derived paths did not make it fail"
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
            probe.note(format!(
                "check '{id}': its criterion already holds on the pre-implementation tree at {short} (it passed there); kept as a regression guard, proven able to fail: it fails when {} is moved aside",
                moved.join(", ")
            ));
        }
    }
    findings
}

enum MutationFinding {
    HostUnproven(String),
    AuthorFinding(String),
}

fn mutation_host_or_author_finding(
    result: &CheckEvidence,
    markers: &mutation::Markers,
    inputs: &[String],
    setup_failure: Option<String>,
    passed_there: &str,
) -> Option<MutationFinding> {
    if let Some(reason) = mutation_host_failure(result, markers, inputs, setup_failure) {
        return Some(MutationFinding::HostUnproven(reason));
    }
    let recreated = recreated_inputs(&result.result, markers, inputs);
    (!recreated.is_empty())
        .then(|| MutationFinding::AuthorFinding(author_recreated_finding(passed_there, &recreated)))
}

fn recreated_inputs(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
) -> Vec<String> {
    recreated_paths(&input_states(result, markers, inputs))
        .into_iter()
        .chain(markers.recreated(result, inputs))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn mutation_host_failure(
    result: &CheckEvidence,
    markers: &mutation::Markers,
    inputs: &[String],
    setup_failure: Option<String>,
) -> Option<String> {
    let states = input_states(&result.result, markers, inputs);
    let status = describe_status(markers.status(&result.result));
    let details = format!(
        "{}; check status/signal: {status}",
        describe_states(&states)
    );
    if result.result.operational_error.is_some() {
        return Some(operational_error_diagnostics(
            &result.result,
            markers,
            inputs,
        ));
    }
    let cause = if markers.guarded(&result.result) {
        Some("the mutation's live-root guard stopped it".to_string())
    } else if let Some(signal) = markers.observed_signal(&result.result) {
        Some(format!("the check wrapper observed signal {signal}"))
    } else if let Some(reason) = setup_failure {
        Some(format!(
            "the mutation run stopped at a setup failure: {reason}"
        ))
    } else {
        let moved = markers.moved(&result.result, inputs);
        let states = markers.states(&result.result, inputs);
        let incomplete_states = moved.iter().any(|path| !states.contains_key(path));
        if incomplete_states {
            Some("the mutation run did not record a state for every moved input".into())
        } else if markers.status(&result.result).is_none() {
            Some("the mutation run did not record its exit status".into())
        } else {
            let unrestored = states
                .iter()
                .any(|(_, state)| state != "restored" && state != "original-recreated");
            unrestored.then(|| "the mutation run left input state unproven".into())
        }
    };
    cause.map(|cause| format!("{cause}; {details}"))
}

fn describe_states(states: &std::collections::BTreeMap<String, String>) -> String {
    format!(
        "input states: {}",
        states
            .iter()
            .map(|(path, state)| format!("{path}={state}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn input_states(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
) -> std::collections::BTreeMap<String, String> {
    let mut states = markers.states(result, inputs);
    for path in markers.moved(result, inputs) {
        states
            .entry(path)
            .or_insert_with(|| "aside-missing: no restoration state was recorded".into());
    }
    for input in inputs {
        states
            .entry(input.clone())
            .or_insert_with(|| "unreported".into());
    }
    states
}

pub(super) fn all_moved_inputs_restored(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
) -> bool {
    let states = markers.states(result, inputs);
    markers
        .moved(result, inputs)
        .iter()
        .all(|path| states.get(path).is_some_and(|state| state == "restored"))
}

fn operational_error_diagnostics(
    result: &CheckResult,
    markers: &mutation::Markers,
    inputs: &[String],
) -> String {
    format!(
        "{}; {}; check status/signal: {}",
        result.operational_error.as_deref().unwrap_or_default(),
        describe_states(&input_states(result, markers, inputs)),
        describe_status(markers.status(result))
    )
}

fn recreated_paths(states: &std::collections::BTreeMap<String, String>) -> Vec<String> {
    states
        .iter()
        .filter_map(|(path, state)| (state == "original-recreated").then_some(path.clone()))
        .collect()
}

fn author_recreated_finding(passed_there: &str, paths: &[String]) -> String {
    format!(
        "{passed_there}; it recreated {} while running, so that data is not evidence; use a movable input that exists under the selected root before the run and make the check fail on its own assertion when it is moved aside",
        paths.join(", ")
    )
}

fn describe_status(status: Option<String>) -> String {
    match status {
        Some(status) => {
            let code = status
                .split(':')
                .nth(1)
                .and_then(|code| code.parse::<i32>().ok());
            let signal = status.split(':').nth(3).unwrap_or("none");
            if signal != "none" {
                format!(
                    "exit status {}, wrapper-observed signal {signal}",
                    code.unwrap_or_default()
                )
            } else {
                format!(
                    "exit status {}",
                    code.map_or(status, |code| code.to_string())
                )
            }
        }
        None => "signal/exit status unavailable (wrapper interrupted before recording it)".into(),
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_prove_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_prove_alias_tests.rs"]
mod alias_tests;
