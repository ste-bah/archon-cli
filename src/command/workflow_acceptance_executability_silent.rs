//! A check that failed on the pre-implementation tree without a verdict
//! (`workflow_acceptance_executability_verdict`) is unproven: the host's,
//! retried. But the base commit never changes, so a retry that meets the
//! same failure would meet it forever, and a task must always stay
//! implementable (Issue 328 round 2). The host remembers strikes: a
//! missing-setup-path strike is keyed by commit, check id and the normalised
//! path from the selected root, so it survives re-authoring; every other
//! strike class is keyed by the executed text and starts afresh when the
//! check changes. The next time it sees the same no-verdict failure again,
//! the check goes to its author as a finding -- through the same path as a
//! check that cannot fail -- to make it fail by its own assertion. An author
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

#[cfg(test)]
use super::silent_strike::normalize_path;
use super::silent_strike::setup_path_signature;
use super::verdict::{Context, no_verdict_cause};
use super::*;
use crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR;
use crate::command::workflow_task_set::passability::evidence::{Redactor, program_output};

/// Bytes of a silent failure's stderr kept as evidence.
const EVIDENCE_BYTES: usize = 800;

/// Where `probe`'s checks run: its site's environment, and `contract`'s
/// declared deliverables.
pub(super) fn context(probe: &HostProbe, contract: &AcceptanceContract) -> Context {
    let at = Context::new(super::sites::listing_environment(probe), contract);
    match scratch_copy_tree(probe, contract, None) {
        Some((depth, roots)) => at.within(depth, &roots),
        None => at,
    }
}

/// The pre-implementation tree is always checked in a copy for a direct
/// site, even though that site's live checks have no copy layout.
pub(super) fn baseline_context(
    probe: &HostProbe,
    contract: &AcceptanceContract,
    layout: Option<&super::hermetic::CopyLayout>,
) -> Context {
    let at = Context::new(super::sites::listing_environment(probe), contract);
    let tree = match probe.site {
        Site::Direct | Site::Hermetic => layout.and_then(|layout| layout.site_tree()),
        Site::Scratch(_) => scratch_copy_tree(probe, contract, layout),
        Site::Unavailable(_) => None,
    };
    match tree {
        Some((depth, roots)) => at.within(depth, &roots),
        None => at,
    }
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
) -> Option<(String, UnprovenCause)> {
    let (contract, result, at) = (contract.clone(), result.clone(), at.clone());
    tokio::task::spawn_blocking(move || silent_failure_cause(&contract, &result, &at))
        .await
        .unwrap_or_else(|error| {
            let why = format!("deciding its verdict failed: {error}");
            Some((why, UnprovenCause::PermanentHost))
        })
}

/// Why `result`, a failed run of a check of `contract`, gave no verdict;
/// `None` when its failure is a verdict.
pub(super) fn silent_failure(
    contract: &AcceptanceContract,
    result: &CheckResult,
    at: &Context,
) -> Option<String> {
    silent_failure_cause(contract, result, at).map(|(why, _)| why)
}

/// Issue 544: whether two copies of one result (its raw capture and its
/// masked copy) give the same text verdicts at `probe`'s site: the
/// no-verdict cause and the setup-path rule. A result store keeps a result
/// only when they agree, so a reused verdict never differs from the raw one.
pub(super) fn verdict_oracle(
    probe: &HostProbe,
    contract: &AcceptanceContract,
) -> super::resume::VerdictOracle {
    let (contract, at) = (contract.clone(), context(probe, contract));
    std::sync::Arc::new(move |raw: &CheckResult, masked: &CheckResult| {
        let cause = |result: &CheckResult| {
            silent_failure_cause(&contract, result, &at).map(|(_, cause)| cause)
        };
        let setup = |result: &CheckResult| setup_path_failure(&contract, result, &at).is_some();
        super::verdict::may_be_host_failure(raw) == super::verdict::may_be_host_failure(masked)
            && cause(raw) == cause(masked)
            && setup(raw) == setup(masked)
    })
}

/// [`silent_failure`] with its cause ([`UnprovenCause`]).
fn silent_failure_cause(
    contract: &AcceptanceContract,
    result: &CheckResult,
    at: &Context,
) -> Option<(String, UnprovenCause)> {
    let entry = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == result.acceptance_id)?;
    let (_, text) = executed_text(entry)?;
    no_verdict_cause(text, result, at)
}

pub(super) async fn setup_path_failure_off_thread(
    contract: &AcceptanceContract,
    result: &CheckResult,
    at: &Context,
) -> Result<Option<String>, String> {
    let (contract, result, at) = (contract.clone(), result.clone(), at.clone());
    tokio::task::spawn_blocking(move || setup_path_failure(&contract, &result, &at))
        .await
        .map_err(|error| format!("deciding its setup-path verdict failed: {error}"))
}

fn setup_path_failure(
    contract: &AcceptanceContract,
    result: &CheckResult,
    at: &Context,
) -> Option<String> {
    let entry = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == result.acceptance_id)?;
    let (_, text) = executed_text(entry)?;
    super::verdict::setup_path::failure(text, result, at)
}

/// The legacy strike file, keyed by the exact check text, id and commit.
fn strike(probe: &HostProbe, commit: &str, contract: &AcceptanceContract, id: &str) -> PathBuf {
    let identity = format!("{commit}\0{id}\0{}", check_text(contract, id));
    probe
        .project
        .join(FREEZE_CACHE_DIR)
        .join("unproven-on-base")
        .join(format!("{}.strike", content_digest(identity.as_bytes())))
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
    let text = check_text(contract, id);
    let identity = format!("{commit}\0{id}\0{text}\0{window}");
    probe
        .project
        .join(FREEZE_CACHE_DIR)
        .join("stalled-on-base")
        .join(format!("{}.strike", content_digest(identity.as_bytes())))
}

fn check_text<'a>(contract: &'a AcceptanceContract, id: &str) -> &'a str {
    (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id)
        .and_then(executed_text)
        .map_or("", |(_, text)| text)
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_silent_test_helpers.rs"]
mod test_helpers;
#[cfg(test)]
pub(super) use test_helpers::settle;

/// [`settle`], recording an unproven check with the cause of `why`.
pub(super) fn settle_as(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    result: &CheckResult,
    (why, cause): (&str, UnprovenCause),
    at: &Context,
) -> Option<String> {
    let id = result.acceptance_id.as_str();
    let short: String = commit.chars().take(12).collect();
    let (environment, forwarded) = super::sites::site_environment(probe);
    let redactor = Redactor::for_environment(environment, &forwarded);
    let evidence = program_output(&result.stderr, &redactor, EVIDENCE_BYTES);
    let signature = setup_path_signature(contract, id, result, why, at);
    let retried = match cause {
        UnprovenCause::TransientHost => "the host retries its inconclusive listing probe",
        UnprovenCause::SiteLacks => {
            "its site lacks what the host has, which is the host's to repair and never its author's; it is probed again once the site provides it"
        }
        UnprovenCause::Host | UnprovenCause::PermanentHost | UnprovenCause::ProgramAbsent => {
            "it is probed again on retry, and goes to its author if it fails so again"
        }
    };
    strike_or_unproven(
        probe,
        commit,
        contract,
        id,
        signature.as_deref(),
        (why, cause),
        || {
            (
                format!(
                    "check '{id}': it cannot be proven on the base commit {short}: {why}. It failed so again when the host ran it on that same commit, and the commit never changes, so no retry can prove it. Its stderr there:\n{evidence}\nMake it fail by its own assertion on that tree -- exercise what the implementation must add -- not through a tool, build or environment it cannot run there"
                ),
                format!(
                    "it failed on the pre-implementation tree at {short}, but {why}, so it gave no verdict there and is not proven able to fail; {retried}. Its stderr there:\n{evidence}"
                ),
            )
        },
    )
}

/// Settle a check timeout using a durable per-check stall strike.
#[cfg(test)]
pub(super) fn settle_timed_out(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
) -> Option<String> {
    settle_timed_out_with_diagnostics(probe, commit, contract, id, "")
}

/// Add run-specific diagnostics without changing the timeout verdict.
pub(super) fn settle_timed_out_with_diagnostics(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
    diagnostics: &str,
) -> Option<String> {
    let short: String = commit.chars().take(12).collect();
    let window = probe.check_bound_secs();
    let why = format!("no output or process-tree activity for {window}s");
    let suffix = if diagnostics.is_empty() {
        String::new()
    } else {
        format!("; {diagnostics}")
    };
    let path = stall_strike(probe, commit, contract, id);
    struck(
        probe,
        &path,
        id,
        (&why, UnprovenCause::Host),
        Credit::None,
        || {
            (
                format!(
                    "check '{id}': it made no progress on the base commit {short} twice: {why}, and so again after a resume. Repair or replace it so that it ends by its own assertion{suffix}"
                ),
                format!(
                    "unproven (timed out): {why} on the pre-implementation tree at {short}; the run pauses resumably since the host may be at fault. If it stalls again after a resume, it goes to its author{suffix}"
                ),
            )
        },
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Credit {
    Saved,
    None,
}

/// Forget the stall strike of check `id` on `commit`: it ran to an end there.
pub(super) fn ran(probe: &HostProbe, commit: &str, contract: &AcceptanceContract, id: &str) {
    let path = stall_strike(probe, commit, contract, id);
    if let Err(error) = archon_workflow::stage_write::remove_file(&path) {
        probe.unproven_error(
            id,
            error.into(),
            format!("stall strike {} could not be cleared", path.display()),
        );
    }
}

/// The finding when `id` already has a no-verdict strike on `commit`;
/// otherwise save one and record `id` unproven. `texts` gives (finding,
/// unproven reason).
pub(super) fn strike_or_unproven(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    id: &str,
    setup_signature: Option<&str>,
    why: (&str, UnprovenCause),
    texts: impl FnOnce() -> (String, String),
) -> Option<String> {
    let path = match setup_signature {
        Some(signature) => probe
            .project
            .join(FREEZE_CACHE_DIR)
            .join("unproven-on-base")
            .join("setup")
            .join(content_digest(format!("{commit}\0{id}").as_bytes()))
            .join(format!("{}.strike", content_digest(signature.as_bytes()))),
        None => strike(probe, commit, contract, id),
    };
    struck(probe, &path, id, why, Credit::Saved, texts)
}

/// The finding when the strike at `path` already exists; otherwise save it
/// durably (written whole, then renamed into place) and record `id` unproven.
fn struck(
    probe: &HostProbe,
    path: &std::path::Path,
    id: &str,
    (why, cause): (&str, UnprovenCause),
    credit: Credit,
    texts: impl FnOnce() -> (String, String),
) -> Option<String> {
    let (finding, unproven) = texts();
    if cause == UnprovenCause::TransientHost {
        probe.unproven_as(id, unproven, cause);
        return None;
    }
    let unsaved = format!("{unproven}; its strike could not be saved, so it stays the host's");
    // Read, decide and write under the run's ownership fence, so a strike is
    // never settled by a stale owner, and written whole then renamed.
    let settled = archon_workflow::stage_write::with_write(|| {
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.is_file() => {
                let mut prefix = Vec::new();
                std::fs::File::open(path)
                    .and_then(|file| file.take(128).read_to_end(&mut prefix))
                    .map_err(|source| archon_workflow::WorkflowError::Io {
                        path: path.to_path_buf(),
                        source,
                    })?;
                // Old binaries saved total-clock cutoffs as strikes. They are
                // not evidence against the author, even if a later failure is
                // non-clock.
                // F6: a site that lacks what the host has is the site's
                // defect, so an earlier strike is no evidence against the
                // author; it stays the host's, earning no credit again.
                if cause == UnprovenCause::SiteLacks {
                    probe.unproven_as(id, unproven, cause);
                    return Ok(None);
                }
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
            Err(source) => {
                return Err(archon_workflow::WorkflowError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|source| archon_workflow::WorkflowError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
        }
        archon_workflow::stage_write::write_bytes(path, why.as_bytes())?;
        if credit == Credit::Saved {
            probe.resume.progress.saved(false);
        }
        probe.unproven_as(id, unproven, cause);
        archon_workflow::WorkflowResult::Ok(None)
    });
    match settled {
        Ok(finding) => finding,
        Err(error) => {
            probe.unproven_error(id, error.into(), unsaved);
            None
        }
    }
}

/// Forget any strike of check `id` on `commit`: it gave a verdict there.
pub(super) fn clear(probe: &HostProbe, commit: &str, contract: &AcceptanceContract, id: &str) {
    let legacy = strike(probe, commit, contract, id);
    let setup = probe
        .project
        .join(FREEZE_CACHE_DIR)
        .join("unproven-on-base")
        .join("setup")
        .join(content_digest(format!("{commit}\0{id}").as_bytes()));
    let cleared = archon_workflow::stage_write::with_write(|| {
        match std::fs::remove_file(&legacy) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(archon_workflow::WorkflowError::Io {
                    path: legacy,
                    source,
                });
            }
        }
        match std::fs::remove_dir_all(&setup) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(archon_workflow::WorkflowError::Io {
                    path: setup,
                    source,
                });
            }
        }
        Ok(())
    });
    if let Err(error) = cleared {
        probe.unproven_error(
            id,
            error.into(),
            "baseline strike could not be cleared".into(),
        );
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_clock_strike_tests.rs"]
mod clock_strike_tests;
#[cfg(test)]
#[path = "workflow_acceptance_executability_silent_fence_tests.rs"]
mod fence_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_setup_roots_tests.rs"]
mod setup_roots_tests;

#[path = "workflow_acceptance_executability_silent_copy_tree.rs"]
mod copy_tree;
use copy_tree::scratch_copy_tree;
