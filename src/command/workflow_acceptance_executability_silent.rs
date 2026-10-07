//! A check that failed on the pre-implementation tree without a verdict
//! (`workflow_acceptance_executability_verdict`) is unproven: the host's,
//! retried. But the base commit never changes, so a retry that meets the
//! same failure would meet it forever, and a task must always stay
//! implementable (Issue 328 round 2). So the host remembers, per base
//! commit and exact check text, that it saw no verdict; the next time it
//! sees none again, the check goes to its author as a finding -- through
//! the same path as a check that cannot fail -- to make it fail by its own
//! assertion. A re-authored check has new text and starts afresh; an author
//! that makes no progress pauses the run, as every re-author does. A check
//! that later gives a verdict there forgets the strike. A check that ran
//! past its per-check bound there is struck the same way (Issue 323).

use std::path::PathBuf;

use super::verdict::{Context, no_verdict};
use super::*;
use crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR;
use crate::command::workflow_task_set::passability::evidence::{Redactor, program_output};

/// Bytes of a silent failure's stderr kept as evidence.
const EVIDENCE_BYTES: usize = 800;

/// Where `probe`'s checks run: its site's environment, and `contract`'s
/// declared deliverables.
pub(super) fn context(probe: &HostProbe, contract: &AcceptanceContract) -> Context {
    Context::new(super::sites::listing_environment(probe), contract)
}

impl HostProbe {
    /// Where this probe's checks run, for judging their results: its
    /// site's own environment and `contract`'s deliverables.
    pub(crate) fn check_site(&self, contract: &AcceptanceContract) -> Context {
        context(self, contract)
    }
}

/// [`silent_failure`] on a blocking thread: deciding it may list a tool
/// (`verdict_subcommand_list`), which must never hold an async worker.
pub(super) async fn silent_failure_off_thread(
    contract: &AcceptanceContract,
    result: &CheckResult,
    at: &Context,
) -> Option<String> {
    let (contract, result, at) = (contract.clone(), result.clone(), at.clone());
    tokio::task::spawn_blocking(move || silent_failure(&contract, &result, &at))
        .await
        .unwrap_or_else(|error| Some(format!("deciding its verdict failed: {error}")))
}

/// Why `result`, a failed run of a check of `contract`, gave no verdict;
/// `None` when its failure is a verdict.
pub(super) fn silent_failure(
    contract: &AcceptanceContract,
    result: &CheckResult,
    at: &Context,
) -> Option<String> {
    let entry = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == result.acceptance_id)?;
    let (_, text) = executed_text(entry)?;
    no_verdict(text, result, at)
}

/// The strike file of check `id` of `contract` on `commit`.
fn strike(probe: &HostProbe, commit: &str, contract: &AcceptanceContract, id: &str) -> PathBuf {
    let text = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id)
        .and_then(executed_text)
        .map_or("", |(_, text)| text);
    let key = content_digest(format!("{commit}\0{id}\0{text}").as_bytes());
    (probe.project.join(FREEZE_CACHE_DIR))
        .join("unproven-on-base")
        .join(format!("{key}.strike"))
}

/// Settle check `id`, which failed on `commit` without a verdict (`why`):
/// unproven the first time (the strike is saved, which a staged freeze
/// counts as progress, so it is retried); the author's finding when an
/// earlier attempt already saw it fail so.
pub(super) fn settle(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    result: &CheckResult,
    why: &str,
) -> Option<String> {
    let id = result.acceptance_id.as_str();
    let short: String = commit.chars().take(12).collect();
    let (environment, forwarded) = super::sites::site_environment(probe);
    let redactor = Redactor::for_environment(environment, &forwarded);
    let evidence = program_output(&result.stderr, &redactor, EVIDENCE_BYTES);
    strike_or_unproven(probe, commit, contract, id, why, || {
        (
            format!(
                "check '{id}': it cannot be proven on the base commit {short}: {why}. It failed so again when the host ran it on that same commit, and the commit never changes, so no retry can prove it. Its stderr there:\n{evidence}\nMake it fail by its own assertion on that tree -- exercise what the implementation must add -- not through a tool, build or environment it cannot run there"
            ),
            format!(
                "it failed on the pre-implementation tree at {short}, but {why}, so it gave no verdict there and is not proven able to fail; it is probed again on retry, and goes to its author if it fails so again. Its stderr there:\n{evidence}"
            ),
        )
    })
}

/// Issue 323: settle check `id` of `contract`, which ran past the probe's
/// per-check bound on `commit`, by the same strike: unproven (timed out)
/// the first time, retried; its author's finding when it times out so again
/// on that same commit -- a retry would only spend the bound again.
pub(super) fn settle_timed_out(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
) -> Option<String> {
    let short: String = commit.chars().take(12).collect();
    let bound = probe.check_bound_secs();
    let why = format!("it ran past the probe's per-check bound of {bound}s");
    strike_or_unproven(probe, commit, contract, id, &why, || {
        (
            format!(
                "check '{id}': it cannot be proven on the base commit {short}: {why} there, and did so again when the host ran it on that same commit, so no retry can prove it. Make it finish within {bound}s on that tree: run only what its criterion needs, not everything around it"
            ),
            format!(
                "unproven (timed out): {why} on the pre-implementation tree at {short}, so the host has no verdict for it; the other checks still ran. It is probed again on retry, and goes to its author if it times out so again"
            ),
        )
    })
}

/// The finding when `id` already has a strike on `commit`; otherwise save
/// one and record `id` unproven. `texts` gives (finding, unproven reason).
fn strike_or_unproven(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
    why: &str,
    texts: impl FnOnce() -> (String, String),
) -> Option<String> {
    let path = strike(probe, commit, contract, id);
    let (finding, unproven) = texts();
    let settled = archon_workflow::stage_write::with_write(|| {
        let exists = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => true,
            Ok(_) => {
                return Err(archon_workflow::WorkflowError::StateCorrupt(format!(
                    "baseline strike {} is not a regular file",
                    path.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(source) => {
                return Err(archon_workflow::WorkflowError::Io {
                    path: path.clone(),
                    source,
                });
            }
        };
        if exists {
            probe.resume.progress.reused(false);
            return Ok(Some(finding));
        }
        archon_workflow::stage_write::write_bytes(&path, why.as_bytes())?;
        probe.resume.progress.saved(false);
        probe.unproven(id, unproven);
        archon_workflow::WorkflowResult::Ok(None)
    });
    match settled {
        Ok(finding) => finding,
        Err(error) => {
            probe.unproven(id, format!("baseline strike could not be settled: {error}"));
            None
        }
    }
}

/// Forget any strike of check `id` on `commit`: it gave a verdict there.
pub(super) fn clear(probe: &HostProbe, commit: &str, contract: &AcceptanceContract, id: &str) {
    let path = strike(probe, commit, contract, id);
    let cleared = archon_workflow::stage_write::remove_file(&path);
    if let Err(error) = cleared {
        probe.unproven(id, format!("baseline strike could not be cleared: {error}"));
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_silent_fence_tests.rs"]
mod fence_tests;
