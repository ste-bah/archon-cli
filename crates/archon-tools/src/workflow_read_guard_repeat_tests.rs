//! Issue-124: an identical failing command repeated with nothing written is
//! cut; varied, passing or re-edited commands are not; a report in the run's
//! artifact directory is not a substantive write for an isolated branch.
use super::{
    DeclaredTargetScope, IDENTICAL_FAILURES_BEFORE_REFUSAL, MAX_NON_WRITING_CALLS_AFTER_WALL,
    READ_WALL_THRASH_MARKER, REPEATED_FAILURE_MARKER, RunStoreScope, WorkflowReadGuard,
    masked_failure,
};
use serde_json::json;

/// The live check, reduced: its status is echoed, so the shell exits 0.
const LIVE_CHECK: &str = "python3 -c \"\nimport json\nreg=json.load(open('.archon/lab/data/registry.json'))\nassert reg\n\"; echo \"EXIT=$?\"";

/// One Bash call through both hooks, as the dispatch drives them: the
/// admission verdict, then — when admitted — the outcome.
fn run(guard: &WorkflowReadGuard, command: &str, exit_zero: bool) -> Option<String> {
    let input = json!({"command": command});
    let verdict = guard.before_tool("Bash", &input);
    if verdict.is_none() {
        guard.after_tool("Bash", &input, exit_zero, "exit");
    }
    verdict
}

fn guard() -> WorkflowReadGuard {
    WorkflowReadGuard::new(40, 20, false, false)
}

#[test]
fn the_fifth_identical_failing_run_is_refused_and_the_session_then_ends() {
    let guard = guard();
    for attempt in 0..IDENTICAL_FAILURES_BEFORE_REFUSAL {
        assert_eq!(run(&guard, "python3 check.py", false), None, "{attempt}");
    }
    // Whitespace is not a different command.
    let refusal = run(&guard, "  python3   check.py ", false).expect("refused");
    assert!(refusal.starts_with(REPEATED_FAILURE_MARKER), "{refusal}");
    // The refusal marked the wall hit: the existing thrash cutoff now counts
    // every call that is not a substantive write, and ends the session.
    for _ in 1..MAX_NON_WRITING_CALLS_AFTER_WALL {
        assert!(guard.terminal_failure().is_none());
        run(&guard, "python3 check.py", false);
    }
    let last = run(&guard, "echo still here", true).expect("terminal");
    assert!(last.starts_with(READ_WALL_THRASH_MARKER), "{last}");
    assert!(guard.terminal_failure().is_some());
}

#[test]
fn the_live_masked_status_loop_is_cut_too() {
    let guard = guard();
    let output = "EXIT=1\nTraceback (most recent call last):\nFileNotFoundError: registry.json\n";
    for _ in 0..IDENTICAL_FAILURES_BEFORE_REFUSAL {
        // Exit 0 from the shell, as the dispatch now reads it back.
        let exit_zero = !masked_failure(LIVE_CHECK, output);
        assert_eq!(run(&guard, LIVE_CHECK, exit_zero), None);
    }
    let refusal = run(&guard, LIVE_CHECK, false).expect("refused");
    assert!(refusal.starts_with(REPEATED_FAILURE_MARKER), "{refusal}");
}

#[test]
fn varied_and_passing_commands_are_never_refused() {
    let guard = guard();
    for round in 0..30 {
        // Thirty different failing commands, and one passing command run
        // thirty times: neither is a repeat of a failure.
        assert_eq!(
            run(&guard, &format!("python3 check.py --case {round}"), false),
            None
        );
        assert_eq!(run(&guard, "python3 check.py --all", true), None);
    }
    assert!(guard.terminal_failure().is_none());
}

#[test]
fn a_pass_or_a_substantive_write_clears_the_count() {
    let guard = guard();
    for _ in 1..IDENTICAL_FAILURES_BEFORE_REFUSAL {
        assert_eq!(run(&guard, "cargo test -p lab", false), None);
    }
    assert_eq!(run(&guard, "cargo test -p lab", true), None);
    for _ in 0..IDENTICAL_FAILURES_BEFORE_REFUSAL {
        assert_eq!(run(&guard, "cargo test -p lab", false), None);
    }
    guard.record_write(b"fn a() {}", b"fn a() { 1 }");
    for _ in 0..IDENTICAL_FAILURES_BEFORE_REFUSAL {
        assert_eq!(run(&guard, "cargo test -p lab", false), None);
    }
    assert!(run(&guard, "cargo test -p lab", false).is_some());
}

#[test]
fn a_masked_status_is_read_only_from_a_trailing_echo_of_it() {
    assert!(masked_failure(LIVE_CHECK, "EXIT=1\n"));
    assert!(masked_failure(LIVE_CHECK, "EXIT=2\nTraceback\n"));
    assert!(!masked_failure(LIVE_CHECK, "EXIT=0\n"));
    assert!(masked_failure("grep -c x f; echo $?", "3\n1\n"));
    assert!(!masked_failure("grep -c x f; echo $?", "3\n0\n"));
    // Not the final statement, or not an echo of `$?`: nothing is inferred.
    assert!(!masked_failure("echo $?; true", "1\n"));
    assert!(!masked_failure("python3 check.py", "EXIT=1\n"));
    assert!(!masked_failure("printf '%s' $?", "1"));
}

fn at_the_wall(scope: RunStoreScope, isolated_worktree: Option<&str>) -> WorkflowReadGuard {
    let mut guard = WorkflowReadGuard::new(0, 20, false, false).with_run_store(scope);
    if let Some(root) = isolated_worktree {
        guard = guard.with_declared_targets(
            DeclaredTargetScope::new(&["src/lib.rs".to_string()], Some(root))
                .in_isolated_worktree(true),
        );
    }
    let wall = guard
        .before_tool("Read", &json!({"file_path": "src/lib.rs"}))
        .expect("the budget is exhausted");
    assert!(wall.starts_with("read budget exhausted"), "{wall}");
    guard
}

fn read_refused(guard: &WorkflowReadGuard) -> bool {
    guard
        .before_tool("Read", &json!({"file_path": "src/lib.rs"}))
        .is_some()
}

#[test]
fn a_run_report_written_from_an_isolated_worktree_does_not_lift_the_wall() {
    let store = tempfile::tempdir().unwrap();
    let run_root = store.path().join("wf-synthetic");
    let worktree = run_root.join("v2/worktrees/item-a/item-a-0");
    std::fs::create_dir_all(run_root.join("artifacts")).unwrap();
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    let scope = RunStoreScope::new(run_root.to_str(), store.path().to_str(), worktree.to_str());
    let guard = at_the_wall(scope.clone(), worktree.to_str());
    let report = run_root.join("artifacts/review-remediate-item-a.md");
    std::fs::write(&report, "blocked").unwrap();
    guard.record_write_at(&report, b"", b"blocked: registry shape");
    assert!(
        read_refused(&guard),
        "a report is not progress on the branch"
    );
    // `..` spelled from the worktree reaches the same directory.
    guard.record_write_at(
        &worktree.join("../../../../artifacts/again.md"),
        b"",
        b"blocked again",
    );
    assert!(read_refused(&guard));
    // The branch's own file is.
    guard.record_write_at(&worktree.join("src/lib.rs"), b"fn a() {}", b"fn a() { 1 }");
    assert!(!read_refused(&guard));

    // Not stamped isolated: the host did not say the artifact directory is
    // outside what the call delivers, so the write counts as it always did.
    let guard = at_the_wall(scope, None);
    guard.record_write_at(&report, b"", b"report again");
    assert!(!read_refused(&guard));
}

#[test]
fn a_run_artifact_is_substantive_for_a_call_working_in_the_project_root() {
    // A serial call works in the tree that holds the store: an artifact there
    // may be the call's own deliverable, and still counts.
    let project = tempfile::tempdir().unwrap();
    let store = project.path().join(".archon/workflows");
    let run_root = store.join("wf-synthetic");
    std::fs::create_dir_all(run_root.join("artifacts")).unwrap();
    let scope = RunStoreScope::new(run_root.to_str(), store.to_str(), project.path().to_str());
    let guard = at_the_wall(scope, project.path().to_str());
    guard.record_write_at(&run_root.join("artifacts/report.md"), b"", b"report");
    assert!(!read_refused(&guard));
}

#[test]
fn scratch_redirections_heredoc_checks_and_identical_rewrites_do_not_reset_it() {
    for check in [
        "python3 check.py 2>/dev/null; echo \"EXIT=$?\"",
        "pytest -q 2>&1 | tee /tmp/log",
        "python3 - <<'EOF'\nassert open('data/registry.json')\nEOF",
    ] {
        let guard = guard();
        for _ in 0..IDENTICAL_FAILURES_BEFORE_REFUSAL {
            assert_eq!(run(&guard, check, false), None, "{check}");
            // Writing the same bytes again changes nothing a check reads.
            guard.record_write_at(std::path::Path::new("/w/src/lib.rs"), b"same", b"same");
        }
        assert!(run(&guard, check, false).is_some(), "{check}");
    }
}

#[test]
fn an_edit_made_from_the_shell_clears_the_count() {
    // The edit-and-rerun loop spelled in Bash: a heredoc rewrite, `sed -i`,
    // a redirection. Each may have changed what the check reads.
    for edit in [
        "cat > src/lib.rs <<'EOF'\nfn a() { 1 }\nEOF",
        "sed -i '' 's/1/2/' src/lib.rs",
        "printf 'fn a() {}' > src/lib.rs",
    ] {
        let guard = guard();
        for _ in 0..IDENTICAL_FAILURES_BEFORE_REFUSAL {
            assert_eq!(run(&guard, "cargo test -p lab", false), None);
        }
        assert_eq!(run(&guard, edit, true), None, "{edit}");
        for _ in 0..IDENTICAL_FAILURES_BEFORE_REFUSAL {
            assert_eq!(run(&guard, "cargo test -p lab", false), None, "{edit}");
        }
    }
    // An inspection in between changes nothing: the fifth is still refused.
    let guard = guard();
    for _ in 0..IDENTICAL_FAILURES_BEFORE_REFUSAL {
        assert_eq!(run(&guard, "cargo test -p lab", false), None);
        assert_eq!(run(&guard, "grep -n fn src/lib.rs", true), None);
    }
    assert!(run(&guard, "cargo test -p lab", false).is_some());
}
