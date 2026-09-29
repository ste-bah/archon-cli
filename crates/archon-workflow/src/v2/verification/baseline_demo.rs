//! A fail-on-old demonstration never contradicts an accepted verdict
//! (Batch K, I3).
//!
//! Live, a verifier proved a remediation's fix by running the frozen check
//! against the PRE-change commit -- `git archive <old> | tar -x -C <dir>` and
//! the check in `<dir>` -- to show it failed before and passes now. The
//! accepted-verdict gate (`agent_adapter_verdict`, Issue-34) counted that
//! failure as the verdict's own evidence against it, and the bounded repair
//! exhausted on a verdict nothing was wrong with.
//!
//! Only a failure of a command run against the change under review may
//! contradict a verdict. The host tells the two apart from the command
//! itself, and verifies what it implies rather than trusting a flag. A
//! failed test command is a demonstration only when ALL of these hold:
//!
//! 1. It has the one narrow shape `demo_shape` reads end to end: `git
//!    archive <rev> | tar -x -C <dir>` into a clean absolute directory
//!    outside the repository, only preparation (`mkdir`/`cp`/`rm`...) around
//!    it, then `cd <dir> && <check>` where the check never leaves `<dir>`
//!    or names the repository. Anything the host cannot read that way --
//!    a second `git`, a subshell, `$`, `-C`, `--manifest-path`, another
//!    `cd` -- is not a demonstration.
//! 2. In the verifier's repository `<rev>` resolves to a commit that is a
//!    strict ancestor of the commit under review (the verification base the
//!    host stamped, else HEAD) with a different tree. `HEAD`, the commit
//!    under review, an unrelated or unknown revision, and a revision with
//!    the same tree are the change, and stay strict.
//! 3. Pass-on-new: another test command in the same result, not itself run
//!    at an older commit, ran the same check and SUCCEEDED. An old commit
//!    that already holds part of the change may fail the check too; the
//!    verdict still rests on the check passing on the change, which this
//!    requires it to show.
//!
//! Every such command is recorded on the result by the host under
//! [`BASELINE_DEMONSTRATIONS_KEY`] (an agent's own value under that key is
//! discarded first), and every gate that weighs a failed test against an
//! accepted verdict -- the in-session re-ask, the post-session demotion, and
//! the base-commit rule's red-test and pre-existing reads -- skips exactly
//! those records. Everything else is judged as before.

use std::path::Path;

use serde_json::{Value, json};

use crate::v2::{WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus};
use crate::write_coordinator::worktree_isolation::run_git;

/// `result.data` key the host records verified demonstrations under.
pub const BASELINE_DEMONSTRATIONS_KEY: &str = "host_baseline_demonstrations";

#[path = "baseline_demo_shape.rs"]
mod demo_shape;
pub(crate) use demo_shape::{Shape, normalized, shape};

fn commit_of(repo: &Path, rev: &str) -> Option<String> {
    let valid = !rev.is_empty()
        && rev.len() <= 200
        && !rev.starts_with('-')
        && !rev.chars().any(char::is_whitespace);
    if !valid {
        return None;
    }
    let spec = format!("{rev}^{{commit}}");
    run_git(&["rev-parse", "--verify", "--quiet", &spec], repo)
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|sha| !sha.is_empty())
}

fn tree_of(repo: &Path, commit: &str) -> Option<String> {
    let spec = format!("{commit}^{{tree}}");
    run_git(&["rev-parse", "--verify", "--quiet", &spec], repo)
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// `rev` as a commit strictly older than `judged` with a different tree.
pub(crate) fn older_commit(repo: &Path, rev: &str, judged: &str) -> Option<String> {
    let old = commit_of(repo, rev)?;
    let judged = commit_of(repo, judged)?;
    if old == judged {
        return None;
    }
    run_git(&["merge-base", "--is-ancestor", &old, &judged], repo).ok()?;
    let (old_tree, judged_tree) = (tree_of(repo, &old)?, tree_of(repo, &judged)?);
    (old_tree != judged_tree).then_some(old)
}

/// Whether `command` ran against an older commit than `judged`: it has
/// the demonstration shape and its revision is strictly older, with another
/// tree. `None` for anything else.
fn older_run(
    repo: &Path,
    judged: &str,
    command: &WorkflowV2CommandRecord,
) -> Option<(Shape, String)> {
    let shape = shape(&command.command, repo)?;
    let old = older_commit(repo, &shape.rev, judged)?;
    Some((shape, old))
}

/// Whether `other` ran exactly `check` against the change under review:
/// either the check alone, word for word, in the checkout (which the check
/// itself was already required never to leave), or the same demonstration
/// shape materializing a commit whose tree IS the judged one.
fn passes_on_change(
    repo: &Path,
    judged: &str,
    other: &WorkflowV2CommandRecord,
    check: &str,
) -> bool {
    if normalized(&other.command) == check && !other.command.contains(['$', '`', '~']) {
        return true;
    }
    shape(&other.command, repo).is_some_and(|at| {
        at.check == check
            && commit_of(repo, &at.rev)
                .and_then(|commit| tree_of(repo, &commit))
                .zip(tree_of(repo, judged))
                .is_some_and(|(tree, judged_tree)| tree == judged_tree)
    })
}

/// The demonstration `command` is, verified in `repo` against `judged`:
/// a failed test of the demonstration shape at an older commit, whose
/// check some OTHER test command in `commands` ran and passed against the
/// change (pass-on-new). Without that pairing the verdict has no evidence
/// the check holds on the change, so the failure is judged as the change's.
pub(crate) fn demonstration(
    repo: &Path,
    judged: &str,
    command: &WorkflowV2CommandRecord,
    commands: &[WorkflowV2CommandRecord],
) -> Option<Value> {
    if command.kind != WorkflowV2CommandKind::Test
        || command.status != WorkflowV2CommandStatus::Failed
        || command.output_summary.trim().is_empty()
    {
        return None;
    }
    let (shape, old) = older_run(repo, judged, command)?;
    let passed_on_change = commands.iter().any(|other| {
        other.kind == WorkflowV2CommandKind::Test
            && other.status == WorkflowV2CommandStatus::Succeeded
            && passes_on_change(repo, judged, other, &shape.check)
    });
    passed_on_change.then(|| {
        json!({ "command": command.command, "revision": old, "judged": judged,
            "check": shape.check })
    })
}

/// Record on `result` every failed test command that is a verified
/// fail-on-old demonstration. Whatever the agent put under the key is
/// dropped first: only the host writes it.
pub(crate) fn classify(
    result: &mut crate::WorkflowV2Result,
    repository_root: Option<&str>,
    judged: Option<&str>,
) {
    if let Some(data) = result.data.as_object_mut() {
        data.remove(BASELINE_DEMONSTRATIONS_KEY);
    }
    let Some(repo) = repository_root.map(Path::new) else {
        return;
    };
    let judged = judged.unwrap_or("HEAD");
    let Some(judged) = commit_of(repo, judged) else {
        return;
    };
    let found: Vec<Value> = result
        .commands_run
        .iter()
        .filter_map(|command| demonstration(repo, &judged, command, &result.commands_run))
        .collect();
    if found.is_empty() {
        return;
    }
    if !result.data.is_object() {
        result.data = json!({});
    }
    if let Some(data) = result.data.as_object_mut() {
        data.insert(BASELINE_DEMONSTRATIONS_KEY.to_string(), Value::Array(found));
    }
}

/// Whether the host recorded `command` as a fail-on-old demonstration.
pub(crate) fn is_baseline_demonstration(
    result: &crate::WorkflowV2Result,
    command: &WorkflowV2CommandRecord,
) -> bool {
    command.status == WorkflowV2CommandStatus::Failed
        && result
            .data
            .get(BASELINE_DEMONSTRATIONS_KEY)
            .and_then(Value::as_array)
            .is_some_and(|found| {
                found.iter().any(|entry| {
                    entry.get("command").and_then(Value::as_str) == Some(&command.command)
                })
            })
}

#[cfg(test)]
#[path = "baseline_demo_tests.rs"]
mod tests;
