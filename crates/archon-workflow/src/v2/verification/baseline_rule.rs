//! The base-commit rule a focused verifier is held to (Obs-31).
//!
//! The write path establishes, before each coder runs, which tests in the
//! task's declared filter were already red on the base commit and who owns
//! each (`write::test_baseline`). This module carries that record to the
//! verifier and enforces it on the verifier's own report:
//!
//! - [`stamp_baseline_tests_input`] puts the task's lists on the verification
//!   item's input under [`BASELINE_TESTS_INPUT_KEY`], where the prompt
//!   builder renders them as a section with the rule.
//! - [`enforce_baseline_tests`] re-reads the verifier's `commands_run` with
//!   the host's own parser: a red test not on the other-owner or ignore
//!   list demotes an accepted verdict whatever the prose says, and a failed
//!   declared command marked `pre_existing` earns no exemption unless every
//!   test its output names is on those lists. "Pre-existing" was exactly the
//!   judgement that accepted wf-caac2ac3's verification with two red tests
//!   nobody owned.
//!
//! One exception to the pre-existing check (Issue-78): a declared filter can
//! be stale — naming a module path the tests are no longer mounted at — so
//! the command matches nothing and exits non-zero. There is then no red test
//! for the claim to name and no wording can prove it, so an evidenced claim
//! on a zero-match command keeps the verdict and is recorded as a `review`
//! gap under its own id instead.
//!
//! A second exception (`baseline_pre_existing`): the host reads only a
//! SUMMARY of the command, and a summary is prose the parser will not mine
//! for test names. When the parser finds no name there, the claim may still
//! be proven from the verifier's TYPED failing names cross-checked against
//! the host's own routing table, and only when every named test is routed to
//! another task.
//!
//! An item with no baseline record (a task that declares no focused tests,
//! or a run that predates the record) is left to the existing rules.
//!
//! Which record (Issue-70): the verifier runs the filter at the checkout's
//! CURRENT head, not at the task's implementation base, and every commit
//! landed between the two can turn a test in another task's file red. The
//! read-only fanout therefore establishes the baseline again at the
//! verification base (`write::test_baseline_verification`) before the
//! items are dispatched, and [`BaselineStamp::for_tasks_at`] prefers the
//! record at that commit; a stamp from the implementation base alone held
//! wf-0ddadd81's docs-only TASK-TRADING-001 to three tests other tasks'
//! commits broke, and looped it through remediation for nothing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::v2::write::test_baseline::{all_records, routed_findings_for_task};
use crate::v2::write::test_baseline_parse::{diagnostic_files, failing_tests};
use crate::v2::{
    BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2FanoutItem, WorkflowV2ResultStore, WorkflowV2Status,
};

use super::baseline_pre_existing::pre_existing_claims;

/// Top-level input key of the stamp. Listed in
/// `reuse_identity::VOLATILE_INPUT_KEYS`: host-derived, never authored.
pub const BASELINE_TESTS_INPUT_KEY: &str = "baseline_tests";

/// Gap id when a red test the task answers for survives an accepted verdict.
pub const BASELINE_RED_TEST_GAP_ID: &str = "baseline_red_test_verification";

/// Gap id when a declared command is excused because its filter matched zero
/// tests: the verdict stands, the stale declaration stays visible. Never the
/// demotion's id — nothing here lowers a status.
pub const BASELINE_ZERO_MATCH_DECLARATION_GAP_ID: &str = "baseline_zero_match_declaration";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OtherOwnerTest {
    pub test_id: String,
    pub owner_task: String,
}

/// A declared command red on the base commit only for error diagnostics in
/// files outside the task's target set (Issue-64). A `pre_existing` claim on
/// it is honoured when every location the verifier's own output names is in
/// `files`; a diagnostic anywhere else — the task's own files — is not
/// pre-existing and refuses the claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreExistingCommand {
    pub command: String,
    /// Repo-relative, sorted.
    pub files: Vec<String>,
}

/// What the verifier is told and held to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineStamp {
    pub base_commit: String,
    /// Declared focused commands that were baselined.
    #[serde(default)]
    pub declared_commands: Vec<String>,
    /// Tests red on the base commit that this task must make pass.
    #[serde(default)]
    pub must_pass: Vec<String>,
    /// Tests red on the base commit owned by another task: exempt.
    #[serde(default)]
    pub other_owner: Vec<OtherOwnerTest>,
    /// Tests red on the base commit the task was told to leave alone: exempt.
    #[serde(default)]
    pub ignored: Vec<String>,
    /// Declared commands the host could not baseline (timed out, unrunnable).
    #[serde(default)]
    pub unbaselined_commands: Vec<String>,
    /// Declared commands red on the base commit for out-of-scope
    /// diagnostics only (Issue-64): exempt while their diagnostics stay
    /// within the listed files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_existing_diagnostics: Vec<PreExistingCommand>,
    /// The canonical task ids this stamp was assembled for: the task under
    /// verification, which the routing table must never name as an owner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<String>,
    /// `base_commit` is the verification base — the head of the checkout
    /// the verifier runs in — rather than the task's implementation base
    /// (Issue-70).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub verification_base: bool,
}

impl BaselineStamp {
    /// Whether `test_id` may stay red under an accepted verdict.
    pub fn exempt(&self, test_id: &str) -> bool {
        self.other_owner.iter().any(|t| t.test_id == test_id)
            || self.ignored.iter().any(|t| t == test_id)
    }

    /// Whether a failed declared `command` whose verifier output is
    /// `output_summary` fails for the out-of-scope diagnostics the baseline
    /// recorded (Issue-64): the command was baselined as such, and every
    /// `--> path` / `Diff in path` location the output names is one of the
    /// recorded files. An output naming no location is taken at the
    /// baseline's word; one naming any other file is not.
    pub fn pre_existing_diagnostics_cover(&self, command: &str, output_summary: &str) -> bool {
        let Some(entry) = self.pre_existing_diagnostics.iter().find(|entry| {
            crate::context::command_matches_declared_focused_test(
                command,
                std::slice::from_ref(&entry.command),
            )
        }) else {
            return false;
        };
        diagnostic_files(output_summary, std::path::Path::new(""))
            .iter()
            .all(|located| {
                entry
                    .files
                    .iter()
                    .any(|file| located == file || located.ends_with(&format!("/{file}")))
            })
    }

    /// The task's baseline, assembled from the records of every branch that
    /// carried one of `task_ids`, at the newest record's base commit, plus
    /// the failures other branches routed to these tasks. `None` when the
    /// run holds nothing for them.
    pub fn for_tasks(store: &WorkflowV2ResultStore, task_ids: &[String]) -> Option<Self> {
        Self::for_tasks_at(store, task_ids, None)
    }

    /// [`Self::for_tasks`], preferring the records established at
    /// `base_commit` when there are any (Issue-70): the verification base
    /// over an older or newer implementation record. Falls back to the
    /// newest record when none was established at that commit.
    pub fn for_tasks_at(
        store: &WorkflowV2ResultStore,
        task_ids: &[String],
        base_commit: Option<&str>,
    ) -> Option<Self> {
        let records: Vec<_> = all_records(store)
            .into_iter()
            .filter(|r| r.canonical_task_ids.iter().any(|id| task_ids.contains(id)))
            .collect();
        let mut stamp = Self::default();
        let chosen = base_commit
            .and_then(|base| records.iter().find(|r| r.base_commit == base))
            .or_else(|| records.first());
        if let Some(chosen) = chosen {
            stamp.base_commit = chosen.base_commit.clone();
            for record in records
                .iter()
                .filter(|r| r.base_commit == stamp.base_commit)
            {
                stamp.must_pass.extend(record.must_pass());
                stamp
                    .other_owner
                    .extend(record.routed.iter().map(|r| OtherOwnerTest {
                        test_id: r.test_id.clone(),
                        owner_task: r.owner_task.clone(),
                    }));
                stamp
                    .ignored
                    .extend(record.ignored.iter().map(|i| i.test_id.clone()));
                for pre in &record.pre_existing {
                    if !stamp
                        .pre_existing_diagnostics
                        .iter()
                        .any(|known| known.command == pre.command)
                    {
                        stamp.pre_existing_diagnostics.push(PreExistingCommand {
                            command: pre.command.clone(),
                            files: pre.files.clone(),
                        });
                    }
                }
                for command in &record.commands {
                    stamp.declared_commands.push(command.command.clone());
                    if command.error.is_some() {
                        stamp.unbaselined_commands.push(command.command.clone());
                    }
                }
            }
        } else if let Some(base) = base_commit {
            stamp.base_commit = base.to_string();
        }
        stamp.verification_base = base_commit.is_some_and(|base| base == stamp.base_commit);
        stamp.tasks = task_ids.to_vec();
        for task in task_ids {
            for finding in routed_findings_for_task(store, task) {
                if let Some(test_id) = finding.get("test_id").and_then(Value::as_str) {
                    stamp.must_pass.push(test_id.to_string());
                }
            }
        }
        for list in [
            &mut stamp.must_pass,
            &mut stamp.ignored,
            &mut stamp.declared_commands,
            &mut stamp.unbaselined_commands,
        ] {
            list.sort();
            list.dedup();
        }
        stamp.other_owner.sort_by(|a, b| a.test_id.cmp(&b.test_id));
        stamp.other_owner.dedup();
        (!records.is_empty() || !stamp.must_pass.is_empty()).then_some(stamp)
    }
}

pub(crate) fn is_focused_verification_call(call_id: &str) -> bool {
    call_id.starts_with("verification-wave-") || call_id.starts_with("review-verification-wave-")
}

/// Stamp the task's baseline onto a focused-verification branch input.
pub fn stamp_baseline_tests_input(call_id: &str, store: &WorkflowV2ResultStore, input: &mut Value) {
    stamp_baseline_tests_input_at(call_id, store, input, None);
}

/// [`stamp_baseline_tests_input`] preferring the record at `base_commit`
/// (Issue-70); an existing stamp is replaced.
pub fn stamp_baseline_tests_input_at(
    call_id: &str,
    store: &WorkflowV2ResultStore,
    input: &mut Value,
    base_commit: Option<&str>,
) {
    if !is_focused_verification_call(call_id) {
        return;
    }
    let task_ids = input
        .get("item")
        .map(crate::v2::review_findings::task_ids_of)
        .unwrap_or_default();
    if task_ids.is_empty() {
        return;
    }
    let Some(stamp) = BaselineStamp::for_tasks_at(store, &task_ids, base_commit) else {
        return;
    };
    if let (Some(object), Ok(value)) = (input.as_object_mut(), serde_json::to_value(&stamp)) {
        object.insert(BASELINE_TESTS_INPUT_KEY.to_string(), value);
    }
}

/// The stamp an input carries, if any.
pub fn stamped(input: &Value) -> Option<BaselineStamp> {
    serde_json::from_value(input.get(BASELINE_TESTS_INPUT_KEY)?.clone()).ok()
}

/// Each item's stamp, keyed by branch id, captured before scheduling.
pub fn baseline_by_item(items: &[WorkflowV2FanoutItem]) -> BTreeMap<String, BaselineStamp> {
    items
        .iter()
        .filter_map(|item| stamped(&item.input).map(|stamp| (item.id.clone(), stamp)))
        .collect()
}

/// Demote every accepted outcome whose own report shows a red test the task
/// answers for, or a pre-existing claim on a declared command that names no
/// exempt test. A claim on a command that matched zero tests is the one
/// exception: it is recorded as a stale declaration and the verdict stands.
pub fn enforce_baseline_tests(
    outcomes: &mut [WorkflowV2BranchOutcome],
    by_item: &BTreeMap<String, BaselineStamp>,
) {
    for outcome in outcomes.iter_mut() {
        if !matches!(
            outcome.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        ) {
            continue;
        }
        let Some(stamp) = by_item.get(&outcome.item_id) else {
            continue;
        };
        let Some(result) = outcome.result.as_mut() else {
            continue;
        };
        let red = red_tests(result, stamp);
        let claims = pre_existing_claims(result, stamp);
        if !claims.zero_match.is_empty() {
            record_zero_match_declaration(result, &claims.zero_match);
        }
        let unproven = claims.unproven;
        if red.is_empty() && unproven.is_empty() {
            continue;
        }
        let mut reasons = Vec::new();
        if !red.is_empty() {
            reasons.push(format!(
                "test(s) in this task's filter fail and are not owned by another task: {}",
                red.join(", ")
            ));
        }
        if !unproven.is_empty() {
            reasons.push(format!(
                "declared command(s) failed under a pre_existing claim whose output names no test on \
                 the other-owner list, so the claim is not accepted: {}",
                unproven.join("; ")
            ));
        }
        let detail = reasons.join(". ");
        result.status = WorkflowV2Status::NeedsReview;
        result.residual_gaps.push(crate::WorkflowV2ResidualGap {
            id: BASELINE_RED_TEST_GAP_ID.to_string(),
            description: format!(
                "the accepted verdict is refused by the base-commit rule (base {}): {detail}. \
                 \"pre-existing\" is not an acceptable reason; the task's baseline lists which \
                 red tests are another task's.",
                stamp.base_commit.chars().take(12).collect::<String>()
            ),
            severity: Some("review".to_string()),
        });
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "accepted verification demoted: a red test the task answers for on its own baseline",
        ));
        let mut data = result.data.as_object().cloned().unwrap_or_default();
        data.insert("baseline_red_tests".to_string(), serde_json::json!(red));
        data.insert(
            "baseline_unproven_pre_existing".to_string(),
            serde_json::json!(unproven),
        );
        data.insert(
            "verification_failure_class".to_string(),
            serde_json::json!("actionable_verification_failure"),
        );
        result.data = Value::Object(data);
        outcome.status = WorkflowV2Status::NeedsReview;
        outcome.failure_kind = Some(BranchFailureKind::Semantic);
    }
}

/// Every test the verifier's own report names as failed — parsed from each
/// command's output by the host, plus the typed `matched_test_check_names.failed`
/// list — minus the exempt ones. Sorted.
fn red_tests(result: &crate::WorkflowV2Result, stamp: &BaselineStamp) -> Vec<String> {
    let mut red: Vec<String> = result
        .commands_run
        .iter()
        .flat_map(|command| failing_tests(&command.output_summary))
        .chain(typed_failed_names(&result.data))
        .filter(|id| !stamp.exempt(id))
        .collect();
    red.sort();
    red.dedup();
    red
}

fn typed_failed_names(data: &Value) -> Vec<String> {
    data.get("matched_test_check_names")
        .and_then(|names| names.get("failed"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

/// Issue-78: the verdict stands, but the stale declaration is never silent —
/// a `review` gap under its own id, evidence naming the excused command(s),
/// and a typed list in `data` so a later reader can correct the declaration
/// that matched nothing. Neither status is touched here.
fn record_zero_match_declaration(result: &mut crate::WorkflowV2Result, commands: &[String]) {
    let listed = commands.join("; ");
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: BASELINE_ZERO_MATCH_DECLARATION_GAP_ID.to_string(),
        description: format!(
            "{} declared test command(s) failed under an attributed pre-existing claim, and the \
             declared filter resolved to no tests at all: {listed}; there is no red test for the \
             claim to name, so the verdict stands — the declaration is stale and must be \
             corrected so the command runs what it names",
            commands.len()
        ),
        severity: Some("review".to_string()),
    });
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "declared test command matched zero tests under an attributed pre-existing claim; verdict kept, stale declaration recorded for review",
    ));
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    data.insert(
        "baseline_zero_match_declarations".to_string(),
        serde_json::json!(commands),
    );
    result.data = Value::Object(data);
}

#[cfg(test)]
#[path = "baseline_rule_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "baseline_rule_verification_tests.rs"]
mod verification_tests;
