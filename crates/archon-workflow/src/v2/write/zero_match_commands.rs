//! Issue-17: scoping the zero-match test rule on a write branch.
//!
//! A test command whose runner output says nothing ran is a verdict only when
//! it is one of the task's DECLARED focused tests, or the envelope presents it
//! as passing evidence (the generic output gate handles the latter). Any other
//! zero match is a `review` residual gap naming the commands: the reviewer
//! sees it, the branch keeps its work.
//!
//! The read guard's own FocusedTests state (the "All declared focused tests
//! have passed" nudge) lives in a per-session task-local in `archon-tools`
//! and is not wired into the write pipeline on this branch, so the host
//! knowledge available here is the declared list on the item and the
//! envelope's own `commands_run`.

use crate::context::{
    command_matches_declared_focused_test, command_output_reports_zero_matched_tests,
};
use crate::error::{WorkflowError, WorkflowResult};
use crate::generated_lifecycle_support::raw_strings;
use crate::v2::{
    WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2Result,
};

/// Gap id prefix for zero-match commands that were neither declared nor
/// presented as passing.
pub(crate) const ZERO_MATCH_TEST_COMMAND_GAP_PREFIX: &str = "zero_match_test_command_";

const DECLARED_FOCUSED_TEST_FIELDS: &[&str] = &[
    "focused_verification",
    "focusedVerification",
    "focused_tests",
    "focusedTests",
];

/// The focused test commands the branch's item declares, verbatim.
pub(super) fn declared_focused_tests(input: &serde_json::Value) -> Vec<String> {
    let item = input.get("item").unwrap_or(input);
    raw_strings(item, DECLARED_FOCUSED_TEST_FIELDS)
}

/// Commands in `result` whose runner output reports zero matched tests,
/// split into (declared or presented as passing, incidental).
fn partition_zero_match(
    result: &WorkflowV2Result,
    declared: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut fatal = Vec::new();
    let mut incidental = Vec::new();
    for command in &result.commands_run {
        if !command_output_reports_zero_matched_tests(&command.command, &command.output_summary) {
            continue;
        }
        let is_declared = command_matches_declared_focused_test(&command.command, declared);
        let presented = command.status == WorkflowV2CommandStatus::Succeeded
            || cited_in_evidence(result, &command.command);
        let bucket = if is_declared || presented {
            &mut fatal
        } else {
            &mut incidental
        };
        if !bucket.contains(&command.command) {
            bucket.push(command.command.clone());
        }
    }
    (fatal, incidental)
}

fn cited_in_evidence(result: &WorkflowV2Result, command: &str) -> bool {
    let needle = normalise(command);
    !needle.is_empty()
        && result.evidence.iter().any(|evidence| {
            normalise(&evidence.summary).contains(&needle)
                || evidence
                    .source
                    .as_deref()
                    .is_some_and(|source| normalise(source).contains(&needle))
        })
}

fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reject the envelope when a declared focused test, or a command cited as
/// evidence, matched zero tests: a filtered run that executed nothing proved
/// nothing about the criterion it was declared for.
pub(super) fn reject_declared_zero_match(
    result: &WorkflowV2Result,
    declared: &[String],
) -> WorkflowResult<()> {
    let (fatal, _) = partition_zero_match(result, declared);
    if fatal.is_empty() {
        return Ok(());
    }
    Err(WorkflowError::StageFailed(format!(
        "agent output not usable: a declared focused test (or a command cited as evidence) \
         matched zero tests, so it proved nothing: {}",
        fatal.join("; ")
    )))
}

/// Record incidental zero-match commands as a review gap and an evidence
/// line. Status and summary are untouched: an exploratory filter that
/// matched nothing is a finding for the reviewer, not a verdict.
pub(super) fn report_incidental_zero_match(
    result: &mut WorkflowV2Result,
    branch_id: &str,
    declared: &[String],
) {
    let (_, incidental) = partition_zero_match(result, declared);
    if incidental.is_empty() {
        return;
    }
    let listed = incidental.join("; ");
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: format!(
            "{ZERO_MATCH_TEST_COMMAND_GAP_PREFIX}{}",
            crate::v2::write::sanitize_v2_path_segment(branch_id)
        ),
        description: format!(
            "{} test command(s) this branch ran matched zero tests and are not declared focused \
             tests nor cited as evidence; they prove nothing and are recorded for review: {listed}",
            incidental.len()
        ),
        severity: Some("review".to_string()),
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        format!("incidental test command(s) matched zero tests: {listed}"),
    ));
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            "zero_match_test_commands".to_string(),
            serde_json::json!(incidental),
        );
    }
}

#[cfg(test)]
#[path = "zero_match_commands_tests.rs"]
mod tests;
