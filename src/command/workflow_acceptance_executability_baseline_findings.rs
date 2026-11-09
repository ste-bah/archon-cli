//! Baseline proof findings for checks that passed on the pre-implementation tree.

use super::*;

/// A pass the acceptance stage would count: exit 0 with real work.
pub fn passed(result: &CheckResult) -> bool {
    result.passed()
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
    findings.extend(
        test_inventory::findings(
            probe,
            baseline,
            contract,
            &run.results.values().cloned().collect::<Vec<_>>(),
        )
        .await,
    );
    let mut passing = Vec::new();
    for reference in refs {
        let id = &reference.acceptance_id;
        if findings.contains_key(id) {
            continue;
        }
        let result = run.results.get(id);
        if result.is_some_and(|result| timed_out(&result.result)) {
            let diagnostic = run_text(baseline.origin, &short, RunText::Timeout);
            if let Some(finding) = silent::settle_timed_out_with_diagnostics(
                probe,
                baseline.origin,
                &baseline.commit,
                contract,
                id,
                &diagnostic,
            ) {
                findings.insert(id.clone(), finding);
            }
            continue;
        }
        if let Some(reason) = run.unrun(id) {
            // What the check's own run caused is its author's (D1).
            let fault = if result.is_none() {
                check_fault::Fault::Availability(run.fault_class)
            } else {
                check_fault::of_result(result.map(|result| &result.result))
            };
            let unproven = run_text(baseline.origin, &short, RunText::Unrun(&reason));
            let at = run_text(baseline.origin, &short, RunText::At);
            let key = (Some(baseline.commit.as_str()), contract, id.as_str());
            if let Some(finding) = check_fault::settle(probe, key, fault, &at, unproven) {
                findings.insert(id.clone(), finding);
            }
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
                        run_text(baseline.origin, &short, RunText::Setup(&error.to_string())),
                    );
                    continue;
                }
            }
            if let Some((why, cause)) =
                silent::silent_failure_off_thread(contract, result, &at).await
            {
                // Issue 328: failing without a verdict proves nothing; failing
                // so again on the same base goes to the author.
                let commit = &baseline.commit;
                let why = (why.as_str(), cause);
                if let Some(finding) =
                    silent::settle_as(probe, baseline.origin, commit, contract, result, why, &at)
                {
                    findings.insert(id.clone(), finding);
                }
                continue;
            }
            silent::clear(probe, &baseline.commit, contract, id);
            if passed(result) {
                probe.passed_on_baseline(id);
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
                origin:
                    crate::command::workflow_task_set::executability::BaselineOrigin::ProbedTree,
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
        if result.is_some_and(|result| timed_out(&result.result)) {
            let diagnostic =
                format!("CHECK_TIMED_OUT on the tree its original failed on ({short})");
            if let Some(finding) = silent::settle_timed_out_with_diagnostics(
                probe,
                BaselineOrigin::ProbedTree,
                tree.commit.as_deref().unwrap_or(&short),
                contract,
                id,
                &diagnostic,
            ) {
                findings.insert(id.clone(), finding);
            }
            continue;
        }
        if let Some(reason) = run.unrun(id) {
            let fault = if result.is_none() {
                check_fault::Fault::Availability(run.fault_class)
            } else {
                check_fault::of_result(result.map(|result| &result.result))
            };
            let unproven = format!(
                "the repair could not be run on the tree its original failed on ({short}: {reason}), so it is not proven to keep that verdict"
            );
            let at = format!("on the tree its original failed on ({short})");
            let key = (tree.commit.as_deref(), contract, id.as_str());
            if let Some(finding) = check_fault::settle(probe, key, fault, &at, unproven) {
                findings.insert(id.clone(), finding);
            }
        } else if let Some(result) = result {
            silent::ran(
                probe,
                tree.commit.as_deref().unwrap_or(&short),
                contract,
                id,
            );
            let at = silent::baseline_context(probe, contract, result.copy_layout.as_ref());
            if let Some((reason, cause)) =
                silent::silent_failure_off_thread(contract, result, &at).await
            {
                probe.unproven_as(
                    id,
                    format!(
                        "the repair gave no verdict on the tree its original failed on ({short}: {reason}), so it is not proven to keep that verdict"
                    ),
                    cause,
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

#[derive(Clone, Copy)]
enum RunText<'a> {
    Timeout,
    Unrun(&'a str),
    At,
    Setup(&'a str),
}

fn run_text(origin: BaselineOrigin, short: &str, text: RunText<'_>) -> String {
    let tree = origin.tree_name();
    match text {
        RunText::Timeout => format!("CHECK_TIMED_OUT on the {tree} at {short}"),
        RunText::Unrun(reason) => format!(
            "it could not be run on the {tree} at {short} ({reason}), so it is not proven able to fail"
        ),
        RunText::At => format!("on the {tree} at {short}"),
        RunText::Setup(error) => format!(
            "it ran on the {tree} at {short}, but the host could not decide whether its setup path was missing ({error})"
        ),
    }
}

#[cfg(test)]
#[test]
#[allow(non_snake_case)]
fn P1_N7_baseline_run_texts_name_each_origin() {
    for origin in [
        BaselineOrigin::RecordedBase,
        BaselineOrigin::HeadFallback,
        BaselineOrigin::ProbedTree,
    ] {
        for text in [
            RunText::Timeout,
            RunText::Unrun("unavailable"),
            RunText::At,
            RunText::Setup("io"),
        ] {
            let text = run_text(origin, "abc", text);
            assert!(
                text.contains(&format!("on the {} at abc", origin.tree_name())),
                "{text}"
            );
            assert_eq!(
                text.contains("pre-implementation tree"),
                origin == BaselineOrigin::RecordedBase
            );
        }
    }
}
