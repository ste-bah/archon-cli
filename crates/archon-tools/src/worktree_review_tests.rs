//! Review-summary tests (#184 M7).

use super::*;

fn review(ahead: usize, behind: usize, files: usize, ins: usize, del: usize) -> WorktreeReview {
    WorktreeReview {
        branch_name: "archon/subagent-1".into(),
        base_branch: "main".into(),
        ahead,
        behind,
        stats: DiffStats {
            files_changed: files,
            insertions: ins,
            deletions: del,
        },
    }
}

#[test]
fn an_untouched_branch_reports_no_changes() {
    let summary = review(0, 0, 0, 0, 0);
    assert!(!summary.has_work());
    assert!(
        summary.describe().contains("no changes"),
        "{}",
        summary.describe()
    );
}

#[test]
fn a_changed_branch_reports_its_diffstat() {
    let described = review(2, 0, 3, 40, 5).describe();
    assert!(described.contains("3 files changed"), "{described}");
    assert!(described.contains("+40 -5"), "{described}");
    assert!(described.contains("2 ahead"), "{described}");
}

/// Being behind the base is the fact that predicts an awkward merge, so it is
/// only mentioned when it is true — a "0 behind" on every row would train the
/// reader to skip the column.
#[test]
fn behind_is_only_mentioned_when_it_is_nonzero() {
    assert!(!review(1, 0, 1, 1, 0).describe().contains("behind"));

    let stale = review(1, 7, 1, 1, 0).describe();
    assert!(stale.contains("7 behind main"), "{stale}");
}

/// Uncommitted work still counts as work. An agent that edited without
/// committing has produced something, and reporting "no changes" would invite
/// discarding it.
#[test]
fn changes_without_commits_still_count_as_work() {
    assert!(review(0, 0, 2, 10, 1).has_work());
}

#[test]
fn commits_without_a_diff_still_count_as_work() {
    assert!(review(1, 0, 0, 0, 0).has_work());
}

#[test]
fn a_single_changed_file_is_not_pluralised() {
    let described = review(1, 0, 1, 2, 0).describe();
    assert!(described.contains("1 file changed"), "{described}");
    assert!(!described.contains("1 files"), "{described}");
}
