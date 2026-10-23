//! Baseline proof findings for checks that passed on the pre-implementation tree.

use super::*;

/// A pass the acceptance stage would count: exit 0 with real work.
pub fn passed(result: &CheckResult) -> bool {
    if let Some(classification) = &result.classification {
        return result.operational_error.is_none() && classification.passed;
    }
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    result.operational_error.is_none()
        && result.exit_code == Some(0)
        && !archon_workflow::acceptance::output_reports_zero_work(&stdout, &stderr)
}

/// Author findings, keyed by id, for every check of `refs` that cannot be
/// shown to fail on the baseline or could not be run there (see the module
/// docs).
pub async fn cannot_fail_findings(
    probe: &HostProbe,
    baseline: &Baseline,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    known: Option<&[super::hermetic::CheckEvidence]>,
) -> BTreeMap<String, String> {
    let mut findings = BTreeMap::new();
    if refs.is_empty() {
        return findings;
    }
    let short: String = baseline.commit.chars().take(12).collect();
    let run = tree_results(probe, baseline, contract, digest, refs, known, false).await;
    let mut passing = Vec::new();
    for reference in refs {
        let id = &reference.acceptance_id;
        let result = run.results.get(id);
        if result.is_some_and(|result| timed_out(&result.result)) {
            // CHECK_TIMED_OUT is the runner's explicit per-check timeout
            // signal. With no separate host-pressure signal, route it to the
            // check's author just like a timeout on the live tree.
            if let Some(finding) =
                crash_findings_at(contract, [&result.expect("timed out result").result], None)
                    .remove(id)
            {
                findings.insert(id.clone(), finding);
            }
            continue;
        }
        if let Some(reason) = run.unrun(id) {
            probe.unproven(
                id,
                format!(
                    "it could not be run on the pre-implementation tree at {short} ({reason}), so it is not proven able to fail"
                ),
            );
        } else if let Some(result) = result {
            // It ran to an end on the base: any earlier stall was not its own.
            silent::ran(probe, &baseline.commit, contract, id);
            let at = silent::baseline_context(probe, contract, result.copy_layout.as_ref());
            // A failed required path outside this disposable tree is a
            // defect in this check's setup, even when the command exited
            // normally. Route that one check to its author; treating it as
            // host-unproven would stop a whole re-author batch and discard
            // unrelated checks that already proved themselves.
            match silent::setup_path_failure_off_thread(contract, result, &at).await {
                Ok(Some(finding)) => {
                    findings.insert(id.clone(), format!("check '{id}': {finding}"));
                    continue;
                }
                Ok(None) => {}
                Err(error) => {
                    probe.unproven(
                        id,
                        format!("it ran on the pre-implementation tree at {short}, but the host could not decide whether its setup path was missing ({error})"),
                    );
                    continue;
                }
            }
            if let Some(why) = silent::silent_failure_off_thread(contract, result, &at).await {
                // Issue 328: failing without a verdict proves nothing; failing
                // so again on the same base goes to the author.
                let commit = &baseline.commit;
                if let Some(finding) = silent::settle(probe, commit, contract, result, &why, &at) {
                    findings.insert(id.clone(), finding);
                }
                continue;
            }
            silent::clear(probe, &baseline.commit, contract, id);
            if passed(result) {
                passing.push(id.clone());
            } else {
                probe.failed_on_baseline(result);
            }
        }
    }
    let copy_layouts = run
        .results
        .iter()
        .filter_map(|(id, evidence)| {
            evidence
                .copy_layout
                .as_ref()
                .map(|layout| (id.clone(), layout.clone()))
        })
        .collect();
    findings.extend(
        prove::prove(
            probe,
            baseline,
            contract,
            digest,
            refs,
            &passing,
            &copy_layouts,
        )
        .await,
    );
    findings
}

/// Findings for every check of `refs` whose original failed its own
/// assertion on `tree` and which passes there (see the module docs).
pub async fn newly_passing_findings(
    probe: &HostProbe,
    tree: &FailedTree,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    known: Option<&[super::hermetic::CheckEvidence]>,
) -> BTreeMap<String, String> {
    let held: Vec<FrozenCommandRef> = (refs.iter())
        .filter(|reference| tree.originals.get(&reference.acceptance_id) == Some(&Original::Failed))
        .cloned()
        .collect();
    let mut findings = BTreeMap::new();
    if held.is_empty() {
        return findings;
    }
    let run = match (known, &tree.commit) {
        (Some(known), _) => TreeRun::known(known, "it did not run at the probe's site"),
        (None, Some(commit)) => {
            let at = Baseline {
                commit: commit.clone(),
                repository: probe.repository.clone(),
            };
            tree_results(probe, &at, contract, digest, &held, None, false).await
        }
        (None, None) => TreeRun::known(&[], "the tree has no commit to copy"),
    };
    let short: String = (tree.commit.as_deref().unwrap_or("the round's tree").chars())
        .take(12)
        .collect();
    for reference in &held {
        let id = &reference.acceptance_id;
        let result = run.results.get(id);
        if let Some(reason) = run.unrun(id) {
            probe.unproven(
                id,
                format!(
                    "the repair could not be run on the tree its original failed on ({short}: {reason}), so it is not proven to keep that verdict"
                ),
            );
        } else if let Some(result) = result {
            let at = silent::baseline_context(probe, contract, result.copy_layout.as_ref());
            if let Some(reason) = silent::silent_failure_off_thread(contract, result, &at).await {
                probe.unproven(
                    id,
                    format!(
                        "the repair gave no verdict on the tree its original failed on ({short}: {reason}), so it is not proven to keep that verdict"
                    ),
                );
            } else if passed(result) {
                findings.insert(
                    id.clone(),
                    format!(
                        "check '{id}': the check it replaces failed its own assertion on the tree at {short}, and this repair passes there; a repair may fix how a check asserts its criterion, never turn a failing product green without a product change: keep every assertion the original made, so that it still fails on that tree"
                    ),
                );
            }
        }
    }
    findings
}
