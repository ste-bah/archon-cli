//! REM-14: every task of the task universe reached an accepted outcome,
//! proven from the host's records -- not from which tasks the script listed.
//!
//! A task's accepted outcome is one of:
//! - its latest pre-review write accepted (or a typed no-op) for it, and a
//!   later pre-review verify accepted for it -- whatever the script did or
//!   did not report, and whoever dispatched it (the host's completion units
//!   run as ordinary writes and verifies before the first review);
//! - it was blocked, and review remediation closed every finding the host's
//!   plan folded in for it (`closure::blocked_tasks_finished`).
//!
//! A task that declares no file it may write is accepted, while no write
//! names it, only on a recorded no-op verify (`host_acceptance`).
//!
//! A task the accounting reports accepted is held to the first by
//! `check_accepted_task`; one it reports blocked to the second by the
//! finding closure, and to a later review of both kinds
//! (`v3_run_outcome_moved`). Every other universe task -- one the script's
//! accounting never mentions -- holds the run by name, whatever the records
//! prove: the prelude folds every task the host completed into the
//! accounting, so a missing one is an accounting the run cannot stand on.

use std::collections::BTreeSet;

use super::keys::TaskKeys;
use super::*;

/// What falls short of an accepted outcome before review for `task`, each
/// with whether it failed on transport; empty when the records prove it:
/// its latest write accepted (or a typed no-op) and a later verify accepted.
/// A universe task that declares no file it may write (`declares_no_file`)
/// has no write to land; for it, and only while no write names it, the host
/// accepts a recorded no-op verify -- the verifier's own statement that its
/// contract holds with nothing changed -- and nothing less.
pub(super) fn host_acceptance(
    task: &str,
    pre_review: &[AuthoredCallFact],
    declares_no_file: bool,
) -> Vec<(String, bool)> {
    let mut short = Vec::new();
    let last = |role: AuthoredCallRole| {
        pre_review
            .iter()
            .enumerate()
            .rev()
            .find(|(_, call)| call.role == role && call.task(task).is_some())
    };
    let Some((write_at, write)) = last(AuthoredCallRole::Write) else {
        if !declares_no_file {
            short.push(("no host write record names it".to_string(), false));
            return short;
        }
        match last(AuthoredCallRole::TaskVerify) {
            Some((_, verify)) => {
                let verified = verify.task(task).expect("filtered on the task");
                if verified.status != WorkflowV2Status::Noop {
                    short.push((
                        format!(
                            "it declares no file to write, which the host accepts only as a recorded no-op verify, and its latest verify `{}` is {:?}",
                            verify.id, verified.status
                        ),
                        verified.transport,
                    ));
                }
            }
            None => short.push((
                "no host write or no-op verify record names it".to_string(),
                false,
            )),
        }
        return short;
    };
    let written = write.task(task).expect("filtered on the task");
    if !is_reusable_status(written.status) {
        short.push((
            format!("its latest write `{}` is {:?}", write.id, written.status),
            written.transport,
        ));
    }
    let Some((verify_at, verify)) = last(AuthoredCallRole::TaskVerify) else {
        short.push(("no host verify record names it".to_string(), false));
        return short;
    };
    let verified = verify.task(task).expect("filtered on the task");
    if !is_reusable_status(verified.status) {
        short.push((
            format!("its latest verify `{}` is {:?}", verify.id, verified.status),
            verified.transport,
        ));
    } else if verify_at < write_at {
        short.push((
            format!("nothing verified it after its latest write `{}`", write.id),
            false,
        ));
    }
    short
}

/// A universe task the universe declares no file it may write for.
fn declares_no_file(task: &str, facts: &AuthoredRunFacts<'_>) -> bool {
    facts.universe_tasks.contains(task) && !facts.writable_tasks.contains(task)
}

/// A task the script reports accepted, held to the host's pre-review record.
pub(super) fn check_accepted_task(
    task: &str,
    pre_review: &[AuthoredCallFact],
    facts: &AuthoredRunFacts<'_>,
    v: &mut Verdict,
) {
    for (clause, transport) in host_acceptance(task, pre_review, declares_no_file(task, facts)) {
        v.block(
            format!("task {task} is reported accepted but {clause}"),
            transport,
        );
    }
}

/// Hold the run on every universe task the accounting never mentions. The
/// prelude folds every task the host completed into the accounting, so one
/// still missing is an accounting the run cannot stand on -- whatever the
/// records show, which the clause states.
pub(super) fn check_universe(
    accounting: &serde_json::Value,
    facts: &AuthoredRunFacts<'_>,
    keys: &TaskKeys<'_>,
    pre_review: &[AuthoredCallFact],
    v: &mut Verdict,
) {
    let mut mentioned: BTreeSet<String> = array(accounting.get("accepted"))
        .iter()
        .filter_map(serde_json::Value::as_str)
        .map(|task| keys.key(task))
        .collect();
    mentioned.extend(
        array(accounting.get("blocked"))
            .iter()
            .filter_map(task_id)
            .map(|task| keys.key(task)),
    );
    for task in facts.universe_tasks {
        if mentioned.contains(task) {
            continue;
        }
        let short = host_acceptance(task, pre_review, declares_no_file(task, facts));
        if short.is_empty() {
            v.block(
                format!(
                    "task {task} of the task universe is missing from the script's accounting, though the host's records show it accepted"
                ),
                false,
            );
            continue;
        }
        let transport = short.iter().any(|(_, transport)| *transport);
        let why: Vec<String> = short.into_iter().map(|(clause, _)| clause).collect();
        v.block(
            format!(
                "task {task} of the task universe reached no accepted outcome: the script's accounting does not name it and {}",
                why.join("; ")
            ),
            transport,
        );
    }
}
