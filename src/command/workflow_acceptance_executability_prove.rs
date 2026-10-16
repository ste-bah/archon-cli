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
    let already = "if the criterion already holds on that tree, the check must still be able to fail: the host runs it again with every data file it names (by a path relative to its working directory) moved aside -- never its own script, a program it runs, a directory it changes into or a build manifest -- so read the files that decide the criterion by those paths";
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
            findings.insert(
                id.clone(),
                format!("{}; {already}; it names none", passed_there(id)),
            );
            continue;
        }
        // One fresh copy per mutated check; a retry meets the same mutation.
        let markers = probe.mutation_markers(baseline, contract, id);
        let inputs = BTreeMap::from([(id.clone(), names)]);
        let mutated = mutated(contract, &inputs, &live, &markers);
        let Ok(digest) = super::contract_digest(&mutated) else {
            probe.unproven(id, "the mutated contract could not be encoded".into());
            continue;
        };
        let ids = BTreeSet::from([id.clone()]);
        let refs = super::refs_for(&mutated, &digest, &ids);
        let run =
            super::repairs::tree_results(probe, baseline, &mutated, &digest, &refs, None, false)
                .await;
        let result = run.results.get(id);
        if result.is_some_and(|result| super::repairs::timed_out(&result.result)) {
            // #356: the mutated check (its nonce kept across retries) is
            // judged on the base like any other stall.
            let commit = &baseline.commit;
            if let Some(finding) = super::silent::settle_timed_out(probe, commit, &mutated, id) {
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
        let host = match result {
            Some(result) if markers.guarded(result) => {
                Some("the mutation's live-root guard stopped it".to_string())
            }
            Some(result) if result.operational_error.is_none() && !markers.restored_all(result) => {
                Some("the run did not restore every input it moved aside (it was killed, or an input was left behind), so its verdict proves nothing".to_string())
            }
            _ if setup_failure.is_some() => setup_failure,
            _ => run.unrun(id),
        };
        if let Some(reason) = host {
            probe.unproven(
                id,
                format!(
                    "it passed on the pre-implementation tree at {short}, and the host could not run it with the data it names moved aside ({reason}), so it is not proven able to fail"
                ),
            );
            continue;
        }
        let result = result.expect("a result when the run completed");
        let at = super::silent::baseline_context(probe, &mutated, result.copy_layout.as_ref());
        super::silent::ran(probe, &baseline.commit, &mutated, id);
        let generated = markers.generated(result);
        let recreated = markers.recreated(result);
        let moved_all = markers.moved(result);
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
        if moved.is_empty() && !recreated.is_empty() {
            findings.insert(
                id.clone(),
                format!(
                    "{}; {already}; it recreated {} while running (parent-directory metadata changed, so that path is not evidence that the check depends on the moved input)",
                    passed_there(id),
                    recreated.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            );
        } else if moved.is_empty() {
            findings.insert(
                id.clone(),
                format!(
                    "{}; {already}; it names none that exists there",
                    passed_there(id)
                ),
            );
        } else if super::baseline::passed(result) {
            if bound_accepted && recreated.is_empty() && !crashed {
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
