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
use crate::v2::verification::path_ownership::{
    DeclaredPathForm, canonical_declared_paths, declared_covers, declared_path_form,
};
use crate::v2::{WorkflowV2BranchOutcome, WorkflowV2FanoutItem};

/// Id prefix of a gap whose every cited path is declared by no task. Carried
/// in the id because a residual gap has no typed field for provenance, and
/// the id is what a dispatch predicate can read back out of plain JSON.
pub const UNOWNED_PATH_GAP_PREFIX: &str = "unowned_path_";
/// The text a flagged gap's description ends with, before the severity the
/// gap had when the host replaced it with `review` and a closing `]`.
pub const FLAGGED_SEVERITY_MARKER: &str = "[severity before flagging: ";

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
    // Issue-88, the rule of this whole module in one place. A gap is flagged
    // ONLY on a positive, proven "no task declares any of these paths".
    // Every other outcome leaves it blocking, because flagging is an EXCUSAL
    // and a wrong excusal hides a real, in-scope failure — the one direction
    // this must never be wrong in. It may downgrade only when ALL of:
    //
    //   - a task universe is present, so "who declares what" has an answer;
    //   - a repository root is known, so both sides reduce to one spelling;
    //   - EVERY declared entry in the universe read as a path (a single
    //     unreadable one might be the path in question, so nothing may be
    //     concluded from its absence);
    //   - the gap cites at least one path that resolves to a real file;
    //   - EVERY cited path canonicalises; and
    //   - no declared path, from this branch or any task, covers any of them.
    //
    // A lookup that merely failed to find an owner is NOT a proof there is
    // none, and never downgrades.
    let Some(universe) = universe else {
        return;
    };
    let Some(declared) = canonical_declared_paths(universe, repository_root) else {
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
            if cited.is_empty() || !all_unowned(&declared, scope, &cited, repository_root) {
                continue;
            }
            gap.id = format!("{UNOWNED_PATH_GAP_PREFIX}{}", gap.id);
            // Issue-117: the severity is replaced, so the one it had is kept
            // in the text, where the residual plan reads it back.
            let original = gap.severity.replace("review".to_string());
            let before = original
                .as_deref()
                .map(str::trim)
                .filter(|severity| !severity.is_empty())
                .map(|severity| format!(" {FLAGGED_SEVERITY_MARKER}{severity}]"))
                .unwrap_or_default();
            gap.description = format!(
                "{} [no task in this run's task universe declares {} as a writable deliverable, \
                 and neither does this branch, so no branch can act on this finding: it is \
                 recorded for review rather than dispatched for remediation, and the defect it \
                 names still has to be fixed by whoever takes ownership of the path]{before}",
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

/// Whether NO declared path covers any cited path — proven, not merely not
/// found. `declared` is already canonical and complete; the branch's own
/// stamped targets are added here because a branch may be granted a target
/// its task body does not spell out.
///
/// Any branch target that cannot be read as a path refuses the whole
/// question: it might be the owner of a cited path.
fn all_unowned(
    declared: &BTreeMap<String, String>,
    scope: &BranchScope,
    cited: &[String],
    repository_root: &Path,
) -> bool {
    let mut own: Vec<String> = Vec::new();
    for target in &scope.targets {
        match declared_path_form(target, repository_root) {
            DeclaredPathForm::Repo(path) => own.push(path),
            DeclaredPathForm::Outside => {}
            DeclaredPathForm::Unusable => return false,
        }
    }
    cited.iter().all(|candidate| {
        !own.iter().any(|target| declared_covers(target, candidate))
            && !declared
                .keys()
                .any(|declared| declared_covers(declared, candidate))
    })
}

/// The unambiguous repository paths `text` names: repo-relative, path-shaped
/// tokens that resolve to a file that exists. Repository-relative by
/// construction, which is the form the declared side is reduced to
/// (Issue-88), so the two are compared in one spelling. A token that looks like a path
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
