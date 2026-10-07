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
//! that later gives a verdict there forgets the strike.
//!
//! A no-progress stall (no output and no process-tree activity for the
//! check's window) is judged the same way, by its own strike, keyed by the
//! same check identity (commit, check text) and its window: the first stall
//! is operational -- the host may be at fault -- so it is unproven and earns
//! no progress credit, and the run pauses resumably. When the same check
//! stalls so again after a resume, it is a check defect and goes to its
//! author through the same repair path. A check that later runs to an end
//! forgets its stall strike. A stall never fails the run and never passes the
//! check.

use std::io::Read;
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
    strike_file(probe, "unproven-on-base", commit, contract, id, "")
}

/// The stall strike of check `id` of `contract` on `commit`: its identity is
/// the no-verdict strike's plus the no-progress window it stalled under, so a
/// longer window is a fresh trial, not a second stall.
fn stall_strike(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
) -> PathBuf {
    let window = probe.check_bound_secs().to_string();
    strike_file(probe, "stalled-on-base", commit, contract, id, &window)
}

fn strike_file(
    probe: &HostProbe,
    kind: &str,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
    salt: &str,
) -> PathBuf {
    let text = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id)
        .and_then(executed_text)
        .map_or("", |(_, text)| text);
    let mut identity = format!("{commit}\0{id}\0{text}");
    if !salt.is_empty() {
        identity.push_str(&format!("\0{salt}"));
    }
    let key = content_digest(identity.as_bytes());
    (probe.project.join(FREEZE_CACHE_DIR))
        .join(kind)
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

/// Settle check `id`, which made no progress (no output, no process-tree
/// activity) for its window on `commit`: unproven the first time, with its
/// stall strike saved but no progress credit, so the run pauses resumably;
/// its author's finding when it stalls so again after a resume.
pub(super) fn settle_timed_out(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
) -> Option<String> {
    let short: String = commit.chars().take(12).collect();
    let window = probe.check_bound_secs();
    let why = format!("no output or process-tree activity for {window}s");
    let path = stall_strike(probe, commit, contract, id);
    struck(probe, &path, id, &why, Credit::None, || {
        (
            format!(
                "check '{id}': it made no progress on the base commit {short} twice: {why}, and so again when the host ran it on that same commit after a resume. A check that waits forever (on input, a socket, a lock or a timer) gives no verdict on any tree. Repair or replace it so that it ends by its own assertion"
            ),
            format!(
                "unproven (timed out): {why} on the pre-implementation tree at {short}; every saved verdict is kept and the run pauses resumably, since the host may be at fault. If it stalls so again on that commit after a resume, it goes to its author"
            ),
        )
    })
}

/// Forget the stall strike of check `id` on `commit`: it ran to an end there.
pub(super) fn ran(probe: &HostProbe, commit: &str, contract: &AcceptanceContract, id: &str) {
    let path = stall_strike(probe, commit, contract, id);
    if let Err(error) = archon_workflow::stage_write::remove_file(&path) {
        probe.unproven(id, format!("stall strike could not be cleared: {error}"));
    }
}

/// Whether saving a first strike is progress for the staged freeze.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Credit {
    /// A no-verdict failure is a finished run: its strike is saved work.
    Saved,
    /// A stall may be the host's fault: no credit, so the run pauses.
    None,
}

/// The finding when `id` already has a no-verdict strike on `commit`;
/// otherwise save one and record `id` unproven. `texts` gives (finding,
/// unproven reason).
fn strike_or_unproven(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
    why: &str,
    texts: impl FnOnce() -> (String, String),
) -> Option<String> {
    let path = strike(probe, commit, contract, id);
    struck(probe, &path, id, why, Credit::Saved, texts)
}

/// The finding when the strike at `path` already exists; otherwise save it
/// durably (written whole, then renamed into place) and record `id` unproven.
fn struck(
    probe: &HostProbe,
    path: &std::path::Path,
    id: &str,
    why: &str,
    credit: Credit,
    texts: impl FnOnce() -> (String, String),
) -> Option<String> {
    let (finding, unproven) = texts();
    let unsaved = format!("{unproven}; its strike could not be saved, so it stays the host's");
    // Read, decide and write under the run's ownership fence, so a strike is
    // never settled by a stale owner, and written whole then renamed.
    let settled = archon_workflow::stage_write::with_write(|| {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.is_file() => {
                let mut prefix = Vec::new();
                std::fs::File::open(path)
                    .and_then(|file| file.take(128).read_to_end(&mut prefix))
                    .map_err(|source| archon_workflow::WorkflowError::io(path, source))?;
                // Old binaries saved total-clock cutoffs as strikes. They are
                // not evidence against the author, even if a later failure is
                // non-clock.
                if !prefix.starts_with(b"it ran past the probe's per-check bound") {
                    probe.resume.progress.reused(false);
                    return Ok(Some(finding));
                }
            }
            Ok(_) => {
                return Err(archon_workflow::WorkflowError::StateCorrupt(format!(
                    "probe strike {} is not a regular file",
                    path.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(archon_workflow::WorkflowError::io(path, source)),
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|source| archon_workflow::WorkflowError::io(dir, source))?;
        }
        archon_workflow::stage_write::write_bytes(path, why.as_bytes())?;
        if credit == Credit::Saved {
            probe.resume.progress.saved(false);
        }
        probe.unproven(id, unproven);
        archon_workflow::WorkflowResult::Ok(None)
    });
    match settled {
        Ok(finding) => finding,
        Err(error) => {
            probe.unproven(id, format!("{unsaved}: {error}"));
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

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_clock_strike_tests.rs"]
mod clock_strike_tests;
#[cfg(test)]
#[path = "workflow_acceptance_executability_silent_fence_tests.rs"]
mod fence_tests;
