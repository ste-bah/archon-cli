//! Checks that could not run are the host's to get running (A1, hole 12).
//!
//! A site that could not be built, or a check that returned no verdict
//! (status `error`), was recorded and the round ended: live, every check of
//! two rounds errored ("native observation guardian failed", a scratch path
//! collision) and nothing acted on it. It is the host's environment, never a
//! task's (Issue-128), so the host repairs it and runs again, in the same
//! round, what gave no verdict, through each repair in turn:
//!
//! 1. a plain re-run: the scratch is rebuilt from scratch and the project's
//!    diverged tracked inputs are restored again before it is built
//!    (`input_divergence`, inside `execute_checks`);
//! 2. a re-run without the run's compiled-artifact cache: a cache left in
//!    a bad state by an interrupted build is not reused.
//!
//! Each repair is recorded. Errors that survive every repair stay on the
//! round: the next round retries, and the loop's progress rule
//! (`acceptance_progress`) decides when that stops.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::{AcceptanceContract, AcceptanceCriterion};
use archon_workflow::{WorkflowResult, WorkflowStore};

use super::exec::{StageContext, checks::Executed, checks::execute_checks};

/// Why a result or a site gave no verdict, normalised so a fresh scratch or
/// evidence path does not read as a different error.
fn errors_of(executed: &Executed) -> BTreeSet<String> {
    let normalize = |text: &str| {
        text.split_whitespace()
            .map(|word| {
                if word.trim_start_matches(['"', '\'', '(']).starts_with('/') {
                    "<path>".to_string()
                } else {
                    word.chars()
                        .map(|c| if c.is_ascii_digit() { '#' } else { c })
                        .collect()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    (executed.site_errors.iter())
        .map(|error| normalize(error))
        .chain(executed.results.iter().filter_map(|result| {
            let error = result.operational_error.as_deref()?;
            Some(format!("{}: {}", result.acceptance_id, normalize(error)))
        }))
        .collect()
}

/// The repairs available, in order; `None` when none is left.
fn repair(context: &mut StageContext, step: usize) -> Option<String> {
    match step {
        0 => Some(
            "re-ran the checks on a freshly built site after restoring the project's diverged inputs"
                .to_string(),
        ),
        1 => {
            let binding = context.binding.as_mut()?;
            let cache = binding.policy.build_cache.take()?;
            Some(format!(
                "re-ran the checks without the run's compiled-artifact cache ({})",
                cache.display()
            ))
        }
        _ => None,
    }
}

/// Execute `selected`, repairing the host environment and re-running what
/// gave no verdict after each repair (see the module docs).
/// `context` keeps the environment the last repair left, so the rest of the
/// round runs where the checks finally ran. Every repair made is appended
/// to `repairs`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_repairing(
    store: &WorkflowStore,
    run_id: &str,
    call_id: &str,
    context: &mut StageContext,
    contract: &AcceptanceContract,
    chain_digest: &str,
    selected: &[&AcceptanceCriterion],
    evidence_dir: &Path,
    repairs: &mut Vec<String>,
) -> WorkflowResult<Executed> {
    let mut executed = execute_checks(
        store,
        run_id,
        call_id,
        context,
        contract,
        chain_digest,
        selected,
        evidence_dir,
    )
    .await?;
    let mut step = 0;
    loop {
        let errors = errors_of(&executed);
        if errors.is_empty() {
            return Ok(executed);
        }
        let Some(made) = repair(context, step) else {
            return Ok(executed);
        };
        step += 1;
        let site_failed = !executed.site_errors.is_empty();
        // A failed site re-runs everything; otherwise only what errored.
        let again: Vec<&AcceptanceCriterion> = (selected.iter().copied())
            .filter(|criterion| {
                site_failed
                    || (executed.results.iter()).any(|result| {
                        result.acceptance_id == criterion.id && result.operational_error.is_some()
                    })
            })
            .collect();
        tracing::warn!(call_id, "acceptance environment repair: {made}");
        repairs.push(format!(
            "{made}: {}",
            errors.into_iter().collect::<Vec<_>>().join("; ")
        ));
        let rerun = execute_checks(
            store,
            run_id,
            call_id,
            context,
            contract,
            chain_digest,
            &again,
            &evidence_dir.join(format!("environment-repair-{step}")),
        )
        .await?;
        executed = merge(selected, executed, rerun, site_failed);
    }
}

/// The round's results after a re-run: the re-run's results replace the
/// ones they re-ran; a failed site's re-run replaces everything.
fn merge(
    selected: &[&AcceptanceCriterion],
    before: Executed,
    rerun: Executed,
    site_failed: bool,
) -> Executed {
    if site_failed {
        return rerun;
    }
    if !rerun.site_errors.is_empty() {
        // The re-run's site failed where the first run's had not: its
        // verdicts stand, and the repair's note records the failure.
        return before;
    }
    let mut by_id: BTreeMap<String, CheckResult> = (before.results.into_iter())
        .map(|result| (result.acceptance_id.clone(), result))
        .collect();
    for result in rerun.results {
        by_id.insert(result.acceptance_id.clone(), result);
    }
    Executed {
        results: (selected.iter())
            .filter_map(|criterion| by_id.remove(&criterion.id))
            .collect(),
        site_errors: Vec::new(),
    }
}
