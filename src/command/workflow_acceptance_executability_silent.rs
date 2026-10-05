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

use std::path::PathBuf;

use super::verdict::{Context, no_verdict};
use super::*;
use crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR;
use crate::command::workflow_task_set::passability::evidence::{Redactor, program_output};

/// Bytes of a silent failure's stderr kept as evidence.
const EVIDENCE_BYTES: usize = 800;

/// Where `probe`'s checks run: its site's search path, and `contract`'s
/// declared deliverables.
pub(super) fn context(probe: &HostProbe, contract: &AcceptanceContract) -> Context {
    let (environment, _) = super::sites::site_environment(probe);
    Context::new(environment.get("PATH").cloned(), contract)
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
    let path = strike(probe, commit, contract, id);
    if path.is_file() {
        probe.resume.progress.reused(false);
        return Some(format!(
            "check '{id}': it cannot be proven on the base commit {short}: {why}. It failed so again when the host ran it on that same commit, and the commit never changes, so no retry can prove it. Its stderr there:\n{evidence}\nMake it fail by its own assertion on that tree -- exercise what the implementation must add -- not through a tool, build or environment it cannot run there"
        ));
    }
    let saved = (path.parent()).is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
        && std::fs::write(&path, why).is_ok();
    if saved {
        probe.resume.progress.saved(false);
    }
    probe.unproven(
        id,
        format!(
            "it failed on the pre-implementation tree at {short}, but {why}, so it gave no verdict there and is not proven able to fail; it is probed again on retry, and goes to its author if it fails so again. Its stderr there:\n{evidence}"
        ),
    );
    None
}

/// Forget any strike of check `id` on `commit`: it gave a verdict there.
pub(super) fn clear(probe: &HostProbe, commit: &str, contract: &AcceptanceContract, id: &str) {
    let _ = std::fs::remove_file(strike(probe, commit, contract, id));
}
