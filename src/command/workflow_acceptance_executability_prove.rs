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

use super::mutation::{mutated, named_inputs};
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
) -> BTreeMap<String, String> {
    let short: String = baseline.commit.chars().take(12).collect();
    let passed_there = |id: &str| {
        format!(
            "check '{id}': it {} at {short}, before any implementation, so it cannot show its criterion false and proves nothing; make it exercise what the implementation must add, so that it fails on that tree and passes only once the criterion holds",
            mutation::CANNOT_FAIL
        )
    };
    let already = "if the criterion already holds on that tree, the check must still be able to fail: name the input files explicitly with paths relative to the check's working directory, so the host can move them aside and verify that the check fails";
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
        let names = (contract.acceptance.iter())
            .chain(&contract.supplementary)
            .find(|entry| &entry.id == id)
            .map(|entry| named_inputs(entry, &probe.project, &baseline.repository))
            .unwrap_or_default();
        if names.is_empty() {
            // Unresolvable inputs never count as moved; a check with no
            // resolvable input routes here so its author can make paths explicit.
            findings.insert(
                id.clone(),
                format!(
                    "{}; {already}; it names none that exists there (no resolvable input)",
                    passed_there(id)
                ),
            );
            continue;
        }
        // One fresh copy per mutated check; a retry meets the same mutation.
        let markers = probe.mutation_markers(baseline, contract, id);
        let inputs = BTreeMap::from([(id.clone(), names.clone())]);
        let mutated = mutated(contract, &inputs, &live, &markers);
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
        match mutation_host_or_author_finding(
            result.expect("no-result case handled above"),
            &markers,
            &names,
            setup_failure,
            &passed_there(id),
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
        let result = result.expect("a result when the run completed");
        let at = super::silent::baseline_context(probe, &mutated, result.copy_layout.as_ref());
        super::silent::ran(probe, &baseline.commit, &mutated, id);
        let generated = markers.generated(result, &names);
        let recreated = recreated_inputs(result, &markers, &names);
        let moved_all = markers.moved(result, &names);
        let moved: Vec<String> = moved_all
            .iter()
            .cloned()
            .into_iter()
            .filter(|path| !generated.contains(path) && !recreated.contains(path))
            .collect();
        // Issue 328: nor did a run that failed for its host (a program that
        // could not start, a tree that did not build) fail on its own terms.
        let crashed = !super::crash_findings(&mutated, [&result.result]).is_empty()
            || super::silent::silent_failure_off_thread(&mutated, result, &at)
                .await
                .is_some();
        if moved.is_empty() {
            findings.insert(
                id.clone(),
                format!(
                    "{}; {already}; it names none that exists there",
                    passed_there(id)
                ),
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
                        "{}; {already}; it still passed with {} moved aside, so it does not depend on them",
                        passed_there(id),
                        moved.join(", ")
                    ),
                );
            }
        } else if crashed {
            findings.insert(
                id.clone(),
                format!(
                    "{}; {already}; with {} moved aside it only crashed, failed to start a program or failed to build, which says nothing about its criterion",
                    passed_there(id),
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
        states.entry(path).or_insert_with(|| "aside-missing".into());
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
        "{passed_there}; it recreated {} while running; stop the check's program from regenerating the named input or stop naming it",
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
