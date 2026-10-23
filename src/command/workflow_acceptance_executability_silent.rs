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

/// Issue 420: the copy `probe`'s checks run in, as (depth of a check's
/// working directory below the copy's top, the names the top holds). A
/// scratch site holds `repo`, `project` and its own directories
/// (`acceptance_scratch::ScratchRoots::prepare`); the probe's own copy
/// holds `repo` and `project` (`HermeticCopy::at`), or keeps a project its
/// repository tracks inside `repo`, at its depth there. The live tree is no
/// copy (`None`): a path above a check's directory may be real there.
fn scratch_copy_tree(
    probe: &HostProbe,
    contract: &AcceptanceContract,
    layout: Option<&super::hermetic::CopyLayout>,
) -> Option<(usize, Vec<String>)> {
    const SCRATCH: &[&str] = &[
        "repo",
        "project",
        "home",
        "tmp",
        "cargo-home",
        "target",
        "project-baseline",
    ];
    match probe.site {
        Site::Scratch(_) => {
            let mut roots: Vec<String> = SCRATCH.iter().map(|root| (*root).to_string()).collect();
            roots.extend(layout.map_or_else(
                || {
                    hermetic::siblings::copied_root_names(
                        &probe.project,
                        &probe.repository,
                        contract,
                    )
                },
                |layout| {
                    layout
                        .siblings
                        .iter()
                        .filter_map(|sibling| {
                            sibling
                                .path
                                .components()
                                .find_map(|component| match component {
                                    std::path::Component::Normal(name) => {
                                        Some(name.to_string_lossy().into_owned())
                                    }
                                    _ => None,
                                })
                        })
                        .collect()
                },
            ));
            Some((1, roots))
        }
        Site::Direct | Site::Hermetic | Site::Unavailable(_) => None,
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
            Some((why, UnprovenCause::Host))
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

/// Settle check `id`, which failed on `commit` without a verdict (`why`):
/// unproven the first time (the strike is saved, which a staged freeze
/// counts as progress, so it is retried); the author's finding when an
/// earlier attempt already saw it fail so.
#[cfg(test)]
pub(super) fn settle(
    probe: &HostProbe,
    commit: &str,
    contract: &AcceptanceContract,
    result: &CheckResult,
    why: &str,
    at: &Context,
) -> Option<String> {
    settle_as(
        probe,
        commit,
        contract,
        result,
        (why, UnprovenCause::Host),
        at,
    )
}

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
                    "it failed on the pre-implementation tree at {short}, but {why}, so it gave no verdict there and is not proven able to fail; it is probed again on retry, and goes to its author if it fails so again. Its stderr there:\n{evidence}"
                ),
            )
        },
    )
}

/// Forget the stall strike of check `id` on `commit`: it ran to an end there.
pub(super) fn ran(probe: &HostProbe, commit: &str, contract: &AcceptanceContract, id: &str) {
    let path = stall_strike(probe, commit, contract, id);
    if let Err(error) = archon_workflow::stage_write::remove_file(&path) {
        probe.unproven(id, format!("stall strike could not be cleared: {error}"));
    }
}

/// The finding when `id` already has a no-verdict strike on `commit`;
/// otherwise save one and record `id` unproven. `texts` gives (finding,
/// unproven reason).
fn strike_or_unproven(
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
    struck(probe, &path, id, why, texts)
}

/// The finding when the strike at `path` already exists; otherwise save it
/// durably (written whole, then renamed into place) and record `id` unproven.
fn struck(
    probe: &HostProbe,
    path: &std::path::Path,
    id: &str,
    (why, cause): (&str, UnprovenCause),
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
                    .map_err(|source| archon_workflow::WorkflowError::Io {
                        path: path.to_path_buf(),
                        source,
                    })?;
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
        probe.resume.progress.saved(false);
        probe.unproven_as(id, unproven, cause);
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
        probe.unproven(id, format!("baseline strike could not be cleared: {error}"));
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_executability_clock_strike_tests.rs"]
mod clock_strike_tests;
#[cfg(test)]
#[path = "workflow_acceptance_executability_silent_fence_tests.rs"]
mod fence_tests;
