//! A finding no branch in the universe can act on (Issue-81).
//!
//! A verifier may report a real defect in a file that NO task declares as a
//! writable deliverable: the path appears in a task body only as a surface
//! observed and reused, and every other task body is silent about it. The
//! finding is honest and the defect is real, but the loop it starts cannot
//! terminate:
//!
//! 1. the verification returns a non-accepted verdict carrying the finding;
//! 2. the host retains the finding as a residual gap on the branch;
//! 3. the lifecycle reads "failed with a gap" as a request for work and
//!    dispatches a write remediation for the task;
//! 4. the write guard correctly refuses the undeclared path, so the branch
//!    returns a no-op saying the file is not its to change;
//! 5. verification runs again, raises the same finding, and round 2 begins.
//!
//! It ends by exhausting the remediation budget, having changed nothing. A
//! blocker no branch can act on is a harness defect: every task must stay
//! implementable.
//!
//! So a gap is flagged here — never dropped, never silenced — when it names
//! at least one repository path and EVERY path it names belongs to no task:
//! not to this branch's declared targets, and not to any task in the
//! universe. The gap keeps its text, gains the paths it is being excused
//! for, drops to `review` severity and takes an id under
//! [`UNOWNED_PATH_GAP_PREFIX`] so the dispatch predicate can skip it while a
//! reader still sees it in full.
//!
//! Everything about this is deliberately conservative, because a wrong
//! downgrade hides a real blocker and that is worse than the loop:
//!
//! - a path counts as cited only when it is unambiguous — a repo-relative,
//!   path-shaped token that resolves to a file that actually exists;
//! - a gap citing no such path is left exactly as it is;
//! - one cited path inside the branch's own scope, or declared by any other
//!   task, leaves the whole gap exactly as it is;
//! - with no task universe the host cannot know who declares what, so
//!   nothing is flagged at all.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::write::test_baseline_owner::{Ownership, ownership};
use crate::v2::{WorkflowV2BranchOutcome, WorkflowV2FanoutItem};

/// Id prefix of a gap whose every cited path is declared by no task. Carried
/// in the id because a residual gap has no typed field for provenance, and
/// the id is what a dispatch predicate can read back out of plain JSON.
pub const UNOWNED_PATH_GAP_PREFIX: &str = "unowned_path_";

/// What a branch was allowed to write, as the ownership test needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchScope {
    /// The branch's canonical task ids.
    pub task_ids: Vec<String>,
    /// Repo-relative declared targets stamped on the item.
    pub targets: Vec<String>,
}

/// Each item's scope, keyed by branch id, captured before scheduling — the
/// same shape and the same moment as the base-commit stamp, because the
/// items are consumed by scheduling and the outcomes carry no scope of
/// their own.
pub fn scope_by_item(items: &[WorkflowV2FanoutItem]) -> BTreeMap<String, BranchScope> {
    items
        .iter()
        .map(|item| {
            let source = item.input.get("item").unwrap_or(&item.input);
            (
                item.id.clone(),
                BranchScope {
                    task_ids: crate::v2::review_findings::task_ids_of(source),
                    targets: crate::v2::call_data::target_files_from_value(source),
                },
            )
        })
        .collect()
}

/// Whether a residual gap, as plain JSON, is one this module flagged.
pub fn gap_is_unowned_path(gap: &Value) -> bool {
    gap.get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| id.starts_with(UNOWNED_PATH_GAP_PREFIX))
}

/// Flag every residual gap whose cited paths are all declared by nobody.
/// Neither `result.status` nor `outcome.status` is touched: the verdict is
/// the verifier's, and this only says who could act on the gap.
pub fn flag_unowned_path_gaps(
    outcomes: &mut [WorkflowV2BranchOutcome],
    by_item: &BTreeMap<String, BranchScope>,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: &Path,
) {
    // No universe means the host cannot know which task declares what, and
    // `ownership` answers `Unowned` for everything: fail closed instead.
    let Some(universe) = universe else {
        return;
    };
    for outcome in outcomes.iter_mut() {
        let Some(scope) = by_item.get(&outcome.item_id) else {
            continue;
        };
        let Some(result) = outcome.result.as_mut() else {
            continue;
        };
        let mut flagged: Vec<String> = Vec::new();
        for gap in result.residual_gaps.iter_mut() {
            if gap.id.starts_with(UNOWNED_PATH_GAP_PREFIX) {
                continue;
            }
            let cited = cited_paths(&gap.description, repository_root);
            if cited.is_empty() || !all_unowned(universe, scope, &cited) {
                continue;
            }
            gap.id = format!("{UNOWNED_PATH_GAP_PREFIX}{}", gap.id);
            gap.severity = Some("review".to_string());
            gap.description = format!(
                "{} [no task in this run's task universe declares {} as a writable deliverable, \
                 and neither does this branch, so no branch can act on this finding: it is \
                 recorded for review rather than dispatched for remediation, and the defect it \
                 names still has to be fixed by whoever takes ownership of the path]",
                gap.description.trim_end(),
                cited.join(", ")
            );
            flagged.extend(cited);
        }
        if flagged.is_empty() {
            continue;
        }
        flagged.sort();
        flagged.dedup();
        record_unowned_paths(result, &flagged);
    }
}

fn all_unowned(universe: &WorkflowV2TaskUniverse, scope: &BranchScope, cited: &[String]) -> bool {
    cited.iter().all(|path| {
        ownership(Some(universe), &scope.task_ids, &scope.targets, path) == Ownership::Unowned
    })
}

/// The unambiguous repository paths `text` names: repo-relative, path-shaped
/// tokens that resolve to a file that exists. A token that looks like a path
/// but names nothing is not a citation — it could be a module path, a glob,
/// a renamed file or prose — and silently excusing a gap over one would be
/// exactly the wrong error.
fn cited_paths(text: &str, repository_root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for raw in text.split([
        ' ', '\t', '\n', '\r', '`', '\'', '"', '(', ')', '[', ']', '{', '}', '<', '>', ',', ';',
        '=',
    ]) {
        // `path/file.rs::symbol` is a location, not a different file.
        let token = raw.split("::").next().unwrap_or(raw);
        let token = token.trim_end_matches(['.', ':', '!', '?']);
        if !is_path_shaped(token) || !repository_root.join(token).is_file() {
            continue;
        }
        paths.push(token.to_string());
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Repo-relative, at least one directory segment, a file name carrying an
/// extension, and nothing that could climb out of the repository.
fn is_path_shaped(token: &str) -> bool {
    !token.starts_with('/')
        && !token.contains("..")
        && token.contains('/')
        && token
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.') && !name.starts_with('.') && name.len() > 2)
}

/// The flagged paths as typed data plus review evidence, so the finding
/// survives into the run's output as something a reader can act on rather
/// than as a gap that quietly stopped mattering.
fn record_unowned_paths(result: &mut crate::WorkflowV2Result, paths: &[String]) {
    result.evidence.push(crate::WorkflowV2Evidence::new(
        crate::WorkflowV2EvidenceKind::Review,
        format!(
            "verification finding(s) name only path(s) no task in the universe declares ({}); \
             recorded for review because no branch could be dispatched to fix them",
            paths.join(", ")
        ),
    ));
    let mut data = result.data.as_object().cloned().unwrap_or_default();
    data.insert(
        "unowned_finding_paths".to_string(),
        serde_json::json!(paths),
    );
    result.data = Value::Object(data);
}

#[cfg(test)]
#[path = "unowned_paths_tests.rs"]
mod tests;
