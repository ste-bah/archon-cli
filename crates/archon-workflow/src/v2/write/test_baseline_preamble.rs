//! What the coder is told about its baseline, in the branch task preamble.
//!
//! Appended next to the landing-policy section (Issue-52), so every session
//! of the branch — first, retry, restart — carries it. The verifier is told
//! the same lists from the same record (`verification::baseline_rule`), so
//! prompt and gate agree on which red tests are the task's.

use super::test_baseline::{BaselineObligation, BranchBaseline};

/// The section for `record`; empty when nothing was baselined.
pub(super) fn preamble(record: &BranchBaseline) -> String {
    if record.is_empty() {
        return String::new();
    }
    let sha: String = record.base_commit.chars().take(12).collect();
    let mut text = format!(
        "\nBaseline tests (the host ran your declared focused test commands on the base commit \
         {sha}, in this worktree, before you started):\n"
    );
    let mine: Vec<String> = record.obligations.iter().map(obligation_label).collect();
    if !record.commands.is_empty() && record.commands.iter().all(|c| c.passed()) {
        text.push_str(
            "- Every test in your declared filter passes on the base commit; any red test after \
             your change is yours.\n",
        );
    }
    if !mine.is_empty() {
        text.push_str(&format!(
            "- Tests already failing on the base commit within your declared filter: {} — these \
             are yours to make pass; their files are in your scope.\n",
            mine.join(", ")
        ));
    }
    if !record.inherited.is_empty() {
        let inherited: Vec<String> = record.inherited.iter().map(obligation_label).collect();
        text.push_str(&format!(
            "- Tests already failing on the base commit in files you declare, found by another \
             task's filter: {} — these are yours to make pass too.\n",
            inherited.join(", ")
        ));
    }
    if !record.routed.is_empty() {
        let routed: Vec<String> = record
            .routed
            .iter()
            .map(|r| format!("{} — owned by {}, ignore", r.test_id, r.owner_task))
            .collect();
        text.push_str(&format!(
            "- Tests already failing on the base commit within your declared filter but owned by \
             another task: {}. Do not edit their files; they are routed to their owner.\n",
            routed.join("; ")
        ));
    }
    if !record.ignored.is_empty() {
        let ignored: Vec<String> = record
            .ignored
            .iter()
            .map(|i| format!("{} ({}; {})", i.test_id, i.file, i.reason))
            .collect();
        text.push_str(&format!(
            "- Tests already failing on the base commit you must leave alone: {}.\n",
            ignored.join("; ")
        ));
    }
    for pre in &record.pre_existing {
        let files: Vec<String> = pre
            .files
            .iter()
            .map(|file| match pre.owners.iter().find(|(f, _)| f == file) {
                Some((_, owner)) => format!("{file} (owned by {owner})"),
                None => format!("{file} (unowned)"),
            })
            .collect();
        text.push_str(&format!(
            "- `{}` already fails at the base commit in {} file(s) outside your target_files: \
             {}; do not fix them — they are reported as pre-existing; you are held only to \
             diagnostics in your own files.\n",
            pre.command,
            pre.files.len(),
            files.join(", ")
        ));
    }
    let unknown: Vec<String> = record
        .commands
        .iter()
        .filter(|c| c.error.is_some())
        .map(|c| format!("`{}` ({})", c.command, c.error.clone().unwrap_or_default()))
        .collect();
    if !unknown.is_empty() {
        text.push_str(&format!(
            "- Declared commands the host could not baseline: {} — run them yourself first and \
             treat what fails as yours unless it is listed above as another task's.\n",
            unknown.join("; ")
        ));
    }
    text.push_str(
        "Your task is not accepted while any test in your declared filter fails, except the ones \
         listed above as owned by another task or to leave alone; \"pre-existing\" is not an \
         acceptable reason, and neither is disabling or deleting the test.\n",
    );
    if !record.pre_existing.is_empty() {
        text.push_str(
            "A declared command listed above as already failing outside your target_files is \
             not yours to make pass: do not edit those files (a write there is refused and \
             would be dropped from your patch); a diagnostic in one of your own files is still \
             yours. If a change there is genuinely required, record it in residual_gaps naming \
             the file and its owner task.\n",
        );
    }
    text
}

fn obligation_label(obligation: &BaselineObligation) -> String {
    match (&obligation.test_id, &obligation.file) {
        (Some(id), Some(file)) => format!("{id} ({file})"),
        (Some(id), None) => format!("{id} (file not resolved)"),
        (None, Some(file)) => format!(
            "`{}` reports error diagnostics in {file}",
            obligation.command
        ),
        (None, None) => format!(
            "`{}` exits non-zero without naming a test",
            obligation.command
        ),
    }
}

#[cfg(test)]
#[path = "test_baseline_preamble_tests.rs"]
mod tests;
