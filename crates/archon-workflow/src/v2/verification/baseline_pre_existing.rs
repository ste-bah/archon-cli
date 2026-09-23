//! Whether a failed declared command's `pre_existing` claim is proven.
//!
//! The host never sees a runner's raw output. The agent runs the declared
//! command and reports a SUMMARY, and a summary is prose. The host's parser
//! (`write::test_baseline_parse`) deliberately refuses to read a test name out
//! of prose — a sentence naming a test is not a verdict on it — so a claim
//! whose only narrative evidence is that summary names no test the host will
//! read, and the rule refused it however complete the evidence behind it was.
//! One branch was refused three times that way: its summary named each failing
//! test in prose, its typed report listed the same names, and the host's own
//! baseline record had already routed every one of them to another task.
//!
//! The parser is left exactly as it is. The claim is instead proven from two
//! records that are not prose, cross-checked against one another — stronger
//! evidence than either alone:
//!
//! - the agent's TYPED declaration of which tests failed, the same field the
//!   verification prompt requires it to return; and
//! - the HOST's own routing table on the branch's baseline stamp, which names
//!   the owning task of every test that was already red on the base commit.
//!
//! A claim reaches the excuse here only when ALL of these hold:
//!
//! - the host's parser found no failing test in the command's own output, so
//!   the typed declaration is the only naming evidence there is;
//! - the stamp records which task is under verification;
//! - the typed field is present, is an array, and every element of it is a
//!   non-empty string;
//! - that array names at least one test;
//! - every name it holds appears in at least one routing entry on the stamp;
//! - every routing entry for every one of those names carries an owner other
//!   than the task under verification.
//!
//! Every other shape keeps today's demotion: a missing, malformed or empty
//! typed field, a name the routing table does not carry at all, and a name
//! routed to the task being verified.

use serde_json::Value;

use super::baseline_rule::BaselineStamp;
use crate::v2::write::test_baseline_parse::failing_tests;
use crate::v2::{WorkflowV2CommandKind, WorkflowV2CommandStatus};

/// Failed declared commands claimed `pre_existing` that the host's parser
/// reads as naming no failing test, split by what became of the claim.
#[derive(Debug, Default)]
pub(super) struct PreExistingClaims {
    /// Nothing ties the failure to another task's test: the claim is refused.
    pub(super) unproven: Vec<String>,
    /// The command matched zero tests, so no red test exists to name
    /// (Issue-78): excused, and recorded as a stale declaration.
    pub(super) zero_match: Vec<String>,
}

/// Classify the failed declared commands carrying a `pre_existing` claim that
/// names no failing test — unless the baseline recorded the command as red
/// for out-of-scope diagnostics and the output locates nothing outside them
/// (Issue-64), which is already an honoured claim and never reaches here.
///
/// Issue-78, the first narrow exception, and the same blind spot the sibling
/// zero-match gate was corrected for: a declared filter can be stale — one
/// module segment short of where the tests are actually mounted — so it
/// matches nothing and the runner exits non-zero. The verifier honestly
/// records the command as failed and attributes it, but its output names no
/// red test because none ran, and no wording could ever prove the claim. Such
/// a command is excused only when BOTH the runner's own summary reports zero
/// matched tests AND the attribution carries its own evidence (the same
/// predicate the accepted-verdict check uses, so the rule that excuses it
/// here is the rule that excuses it there).
///
/// The second exception is [`routed_claim_holds`]: tests did run and the
/// verifier declared their names TYPED, and the host's own routing table
/// already answers who owns each of them. Every other shape — a genuine red
/// test with nothing named anywhere, an unevidenced claim, a typed list the
/// routing table does not fully cover — still refuses the claim.
pub(super) fn pre_existing_claims(
    result: &crate::WorkflowV2Result,
    stamp: &BaselineStamp,
) -> PreExistingClaims {
    let mut claims = PreExistingClaims::default();
    for command in result
        .commands_run
        .iter()
        .filter(|command| command.kind == WorkflowV2CommandKind::Test)
        .filter(|command| command.status == WorkflowV2CommandStatus::Failed && command.pre_existing)
        .filter(|command| {
            crate::context::command_matches_declared_focused_test(
                &command.command,
                &stamp.declared_commands,
            )
        })
        .filter(|command| failing_tests(&command.output_summary).is_empty())
        .filter(|command| {
            !stamp.pre_existing_diagnostics_cover(&command.command, &command.output_summary)
        })
    {
        if crate::context::command_output_reports_zero_matched_tests(
            &command.command,
            &command.output_summary,
        ) && super::is_evidenced_pre_existing_failure(command)
        {
            claims.zero_match.push(command.command.clone());
        } else if !routed_claim_holds(&result.data, stamp) {
            claims.unproven.push(command.command.clone());
        }
    }
    claims
}

/// The claim is proven when the verifier's typed failing names are
/// well-formed and every one of them is routed to another task.
fn routed_claim_holds(data: &Value, stamp: &BaselineStamp) -> bool {
    declared_failed_names(data).is_some_and(|names| routed_to_other_tasks(&names, stamp))
}

/// The typed failing names the verifier declared, read strictly: `Some` only
/// for an array of non-empty strings. Any other shape is malformed, and a
/// malformed declaration proves nothing.
pub(super) fn declared_failed_names(data: &Value) -> Option<Vec<String>> {
    let entries = data
        .get("matched_test_check_names")?
        .get("failed")?
        .as_array()?;
    let mut names = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry.as_str()?.trim();
        if name.is_empty() {
            return None;
        }
        names.push(name.to_string());
    }
    Some(names)
}

/// Whether the host's routing table carries every name in `names` and routes
/// each of them to a task other than the one under verification.
pub(super) fn routed_to_other_tasks(names: &[String], stamp: &BaselineStamp) -> bool {
    if names.is_empty() || stamp.tasks.is_empty() {
        return false;
    }
    names.iter().all(|name| {
        let mut routed = stamp
            .other_owner
            .iter()
            .filter(|entry| &entry.test_id == name)
            .peekable();
        routed.peek().is_some() && routed.all(|entry| !stamp.tasks.contains(&entry.owner_task))
    })
}

#[cfg(test)]
#[path = "baseline_pre_existing_tests.rs"]
mod tests;
