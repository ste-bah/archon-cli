//! A probe run the host could not complete is the host's to repair before
//! it is anyone's finding (A4, A5).
//!
//! A check that gave no verdict on a probed tree (the copy or scratch could
//! not be built, or the run itself errored) says nothing about the check, so
//! its author cannot fix it. The probe first repairs its own environment,
//! the way the acceptance stage does (`workflow_live_v3_acceptance_env`),
//! and runs again what gave no verdict after each repair in turn:
//!
//! 1. a plain re-run on a freshly built copy or scratch;
//! 2. when the site has one, a re-run without the run's compiled-artifact
//!    cache: a cache left in a bad state by an interrupted build is not
//!    reused.
//!
//! Each repair is recorded as a diagnostic (the freeze prints it, a round's
//! repair record keeps it). Only what still gives no verdict after every
//! repair becomes the author's finding, with the repair log attached.

use super::hermetic::Unrun;
use super::sites::run_at;
use super::*;

/// Verdicts on one tree, and how the host got them.
pub(super) struct TreeRun {
    pub(super) results: BTreeMap<String, CheckResult>,
    /// Why the last attempt could not run at all.
    why: String,
    /// Every environment repair made, oldest first.
    repairs: Vec<String>,
}

impl TreeRun {
    /// Verdicts already observed, no run made.
    pub(super) fn known(results: &[CheckResult], why: &str) -> Self {
        Self {
            results: (results.iter())
                .map(|result| (result.acceptance_id.clone(), result.clone()))
                .collect(),
            why: why.to_string(),
            repairs: Vec::new(),
        }
    }

    fn error(&self, id: &str) -> Option<String> {
        match self.results.get(id) {
            Some(result) if result.operational_error.is_none() => None,
            other => Some(
                other
                    .and_then(|result| result.operational_error.clone())
                    .unwrap_or_else(|| self.why.clone()),
            ),
        }
    }

    /// Why `id` gave no verdict after every repair, the repairs included;
    /// `None` when it gave one.
    pub(super) fn unrun(&self, id: &str) -> Option<String> {
        let error = self.error(id)?;
        if self.repairs.is_empty() {
            return Some(error);
        }
        Some(format!(
            "{error}; it still gave no verdict after the host repaired its environment: {}",
            self.repairs.join("; then ")
        ))
    }
}

/// The next repair of the probe's environment, `None` when none is left.
/// `cold` becomes true once the build cache is set aside.
fn repair(probe: &HostProbe, step: usize, cold: &mut bool) -> Option<String> {
    match step {
        0 => Some("re-ran it on a freshly built copy of the tree".to_string()),
        1 => match &probe.site {
            Site::Scratch(binding) if !*cold => {
                let cache = binding.policy.build_cache.as_ref()?;
                *cold = true;
                Some(format!(
                    "re-ran it without the run's compiled-artifact cache ({})",
                    cache.display()
                ))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Verdicts per id of `refs` on `tree`, repairing the host environment and
/// running again what gave no verdict (see the module docs). `known` holds
/// verdicts already observed on that very tree.
pub(super) async fn tree_results(
    probe: &HostProbe,
    tree: &Baseline,
    contract: &AcceptanceContract,
    digest: &str,
    refs: &[FrozenCommandRef],
    known: Option<&[CheckResult]>,
    cold: bool,
) -> TreeRun {
    let mut run = TreeRun::known(&[], "");
    run.results = (known.into_iter().flatten())
        .filter(|result| result.operational_error.is_none())
        .map(|result| (result.acceptance_id.clone(), result.clone()))
        .collect();
    let short: String = tree.commit.chars().take(12).collect();
    let (mut cold, mut step) = (cold, 0);
    loop {
        let pending: Vec<FrozenCommandRef> = (refs.iter())
            .filter(|reference| run.error(&reference.acceptance_id).is_some())
            .cloned()
            .collect();
        if pending.is_empty() {
            break;
        }
        let attempted = !run.why.is_empty()
            || (pending.iter()).any(|reference| run.results.contains_key(&reference.acceptance_id));
        if attempted {
            let Some(made) = repair(probe, step, &mut cold) else {
                break;
            };
            step += 1;
            let errors = (pending.iter())
                .map(|reference| {
                    let id = &reference.acceptance_id;
                    format!("{id}: {}", run.error(id).unwrap_or_default())
                })
                .collect::<Vec<_>>()
                .join(", ");
            let entry = format!("{made} ({errors})");
            probe.note(format!(
                "host-environment repair of the probe at {short}: {entry}"
            ));
            run.repairs.push(entry);
        }
        match run_at(probe, tree, contract, digest, &pending, cold).await {
            Ok(ran) => {
                for result in ran {
                    run.results.insert(result.acceptance_id.clone(), result);
                }
                // A pending check the run returned nothing for gave no
                // verdict either.
                run.why = "the run returned no result for it".to_string();
            }
            Err(Unrun(reason)) => run.why = reason,
        }
    }
    run
}
