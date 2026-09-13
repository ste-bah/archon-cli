use super::*;
use std::process::Command;

fn sh(args: &[&str], cwd: &Path) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn repo_with_worktrees(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let canonical = root.join("repo");
    std::fs::create_dir_all(&canonical).unwrap();
    sh(&["init", "-q"], &canonical);
    sh(&["config", "user.email", "t@example.invalid"], &canonical);
    sh(&["config", "user.name", "t"], &canonical);
    std::fs::write(canonical.join("lib.rs"), "fn a() {}\n").unwrap();
    std::fs::write(canonical.join(".gitignore"), "target/\n").unwrap();
    sh(&["add", "."], &canonical);
    sh(&["commit", "-qm", "base"], &canonical);
    let first = root.join("ws-1");
    let second = root.join("ws-2");
    sh(
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            first.to_str().unwrap(),
            "HEAD",
        ],
        &canonical,
    );
    sh(
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            second.to_str().unwrap(),
            "HEAD",
        ],
        &canonical,
    );
    (canonical, first, second)
}

#[test]
fn captures_diff_of_a_timed_out_worktree_and_reapplies_it() {
    let temp = tempfile::tempdir().unwrap();
    let (_canonical, first, second) = repo_with_worktrees(temp.path());
    std::fs::write(first.join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    std::fs::write(first.join("new.rs"), "pub fn c() {}\n").unwrap();
    std::fs::create_dir_all(first.join("target")).unwrap();
    std::fs::write(first.join("target/junk"), "x").unwrap();
    let run_root = temp.path().join("run");
    let partial = capture_partial_work(&first, &run_root, "agents-2", "agents-2-0")
        .unwrap()
        .expect("changes exist");
    assert_eq!(
        partial.files,
        vec!["lib.rs".to_string(), "new.rs".to_string()]
    );
    assert!(
        partial
            .patch_path
            .starts_with(run_root.join("write-coordination/stages/agents-2/partial"))
    );
    assert!(partial.bytes > 0);
    apply_partial_work(&second, &partial).unwrap();
    assert_eq!(
        std::fs::read_to_string(second.join("lib.rs")).unwrap(),
        "fn a() {}\nfn b() {}\n"
    );
    assert_eq!(
        std::fs::read_to_string(second.join("new.rs")).unwrap(),
        "pub fn c() {}\n"
    );
    assert!(!second.join("target/junk").exists());
    let prompt = with_resume_preamble("do the task", Some(&partial));
    assert!(prompt.contains("lib.rs, new.rs") && prompt.ends_with("do the task"));
}

#[test]
fn an_untouched_worktree_has_no_partial_work() {
    let temp = tempfile::tempdir().unwrap();
    let (_c, first, _s) = repo_with_worktrees(temp.path());
    assert!(
        capture_partial_work(&first, &temp.path().join("run"), "s", "i")
            .unwrap()
            .is_none()
    );
}

#[test]
fn latest_partial_is_found_by_canonical_task_id() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let older = temp.path().join("older.patch");
    let newer = temp.path().join("newer.patch");
    std::fs::write(&older, "o").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&newer, "n").unwrap();
    let outcome = |item: &str, patch: Option<&Path>, task: &str| {
        let mut result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            data: serde_json::json!({"canonical_task_ids": [task]}),
            ..WorkflowV2Result::default()
        };
        if let Some(patch) = patch {
            record_partial_work(
                &mut result,
                &PartialWork {
                    patch_path: patch.to_path_buf(),
                    files: vec!["f".into()],
                    bytes: 1,
                    baseline_commit: "c".into(),
                },
            );
        }
        crate::v2::WorkflowV2BranchOutcome {
            item_id: item.into(),
            role: "coder".into(),
            status: WorkflowV2Status::NeedsReview,
            result: Some(result),
            error: None,
            failure_kind: None,
            item_input_hash: None,
            completion_evidence: Vec::new(),
        }
    };
    store
        .save_branch_outcome("agents-2", &outcome("agents-2-0", Some(&older), "TASK-001"))
        .unwrap();
    store
        .save_branch_outcome("agents-4", &outcome("agents-4-0", None, "TASK-001"))
        .unwrap();
    store
        .save_branch_outcome("agents-5", &outcome("agents-5-0", Some(&newer), "TASK-001"))
        .unwrap();
    store
        .save_branch_outcome("agents-6", &outcome("agents-6-0", Some(&older), "TASK-002"))
        .unwrap();
    let found = latest_partial_for_tasks(&store, &["TASK-001".to_string()]).expect("found");
    assert_eq!(found.patch_path, newer);
    assert!(latest_partial_for_tasks(&store, &["TASK-009".to_string()]).is_none());
    assert!(!branch_keeps_partial_work(
        &WorkflowV2Result {
            status: WorkflowV2Status::Accepted,
            ..Default::default()
        },
        false
    ));
    assert!(branch_keeps_partial_work(
        &WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            ..Default::default()
        },
        false
    ));
    assert!(!branch_keeps_partial_work(
        &WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            ..Default::default()
        },
        true
    ));
}

#[test]
fn a_partial_that_no_longer_applies_leaves_the_workspace_clean() {
    let temp = tempfile::tempdir().unwrap();
    let (_canonical, first, second) = repo_with_worktrees(temp.path());
    std::fs::write(first.join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let partial = capture_partial_work(&first, &temp.path().join("run"), "s", "i")
        .unwrap()
        .unwrap();
    // The second workspace diverged on the same lines, so the patch conflicts.
    std::fs::write(second.join("lib.rs"), "fn a() {}\nfn z() {}\n").unwrap();
    sh(&["commit", "-qam", "diverged"], &second);
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        data: serde_json::json!({"canonical_task_ids": ["TASK-001"]}),
        ..WorkflowV2Result::default()
    };
    record_partial_work(&mut result, &partial);
    store
        .save_branch_outcome(
            "s",
            &crate::v2::WorkflowV2BranchOutcome {
                item_id: "i".into(),
                role: "coder".into(),
                status: WorkflowV2Status::NeedsReview,
                result: Some(result),
                error: None,
                failure_kind: None,
                item_input_hash: None,
                completion_evidence: Vec::new(),
            },
        )
        .unwrap();
    let input = serde_json::json!({"item": {"item_id": "i2", "canonical_task_ids": ["TASK-001"]}});
    assert!(resume_into_workspace(&store, None, &input, &second).is_none());
    assert_eq!(
        std::fs::read_to_string(second.join("lib.rs")).unwrap(),
        "fn a() {}\nfn z() {}\n"
    );
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&second)
        .output()
        .unwrap();
    assert!(
        status.stdout.is_empty(),
        "conflict residue: {}",
        String::from_utf8_lossy(&status.stdout)
    );
}

#[test]
fn the_host_preamble_states_the_budget_and_the_write_first_rule() {
    let plain = with_host_preamble("do the task", None, None);
    assert_eq!(plain, "do the task");
    let budgeted = with_host_preamble(
        "do the task",
        Some(std::time::Duration::from_secs(5400)),
        None,
    );
    assert!(budgeted.starts_with("Time budget: this call has 90 minutes"));
    assert!(budgeted.contains("Write the deliverable files first"));
    assert!(budgeted.ends_with("\n\ndo the task"));
    let partial = PartialWork {
        patch_path: "p".into(),
        files: vec!["a.rs".into()],
        bytes: 1,
        baseline_commit: "c".into(),
    };
    let both = with_host_preamble(
        "do the task",
        Some(std::time::Duration::from_secs(61)),
        Some(&partial),
    );
    assert!(both.starts_with("Time budget: this call has 2 minutes"));
    assert!(both.contains("has been applied to this workspace: a.rs"));
}

#[test]
fn write_read_set_wave_collection_preserves_clean_worktree_reads_in_saved_outcome() {
    let temp = tempfile::tempdir().unwrap();
    let (_, first, _) = repo_with_worktrees(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let sidecar = crate::v2::write_read_set::path(&store, "agents-2-0");
    std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
    std::fs::write(&sidecar, "{\"path\":\"lib.rs\",\"offset\":0,\"limit\":1}\n").unwrap();
    let branch = super::super::worktree::CompletedWorktreeBranch {
        item_id:"agents-2-0".into(), role:"coder".into(), item_input_hash:None,
        result:WorkflowV2Result {
            status:WorkflowV2Status::NeedsReview,
            data:serde_json::json!({"canonical_task_ids":["TASK-001"],"failure_kind":"execution"}),
            ..Default::default()
        },
        manifest:None, pre_hashes:None, workspace_root:first.clone(),
    };
    super::super::worktree_wave::collect_worktree_wave_artifacts(
        vec![branch], &store, "agents-2", &temp.path().join("run"),
    ).unwrap();
    let saved = store.load_branch_outcomes().unwrap();
    let result = saved[0].result.as_ref().unwrap();
    assert!(result.data.get("partial_work").is_none());
    assert_eq!(result.data["workflow_read_set"][0]["path"], "lib.rs");
    assert!(result.evidence.iter().any(|e| e.summary.contains("lib.rs")));
    let status = Command::new("git").args(["status", "--porcelain"]).current_dir(&first).output().unwrap();
    assert!(status.status.success() && status.stdout.is_empty());
}

/// The prompt says the number the host will enforce, not the larger total.
///
/// Live: a coder was told "240 minutes" from the call's total budget while
/// `host_call_timeout_secs` ended its session at 7200 s.
#[test]
fn the_rendered_budget_is_the_smaller_of_the_host_cutoff_and_the_call_total() {
    use std::time::Duration;
    let host_call_timeout = Some(Duration::from_secs(7200));
    let call_total = Some(Duration::from_secs(14_400));
    let effective = effective_call_budget(host_call_timeout, call_total, Duration::ZERO);
    assert_eq!(effective, host_call_timeout);
    let text = with_host_preamble("do the task", effective, None);
    assert!(text.starts_with("Time budget: this call has 120 minutes"), "{text}");
    assert!(!text.contains("240 minutes"));
    // Late in the call the total is what is left, and it wins once smaller.
    assert_eq!(
        effective_call_budget(host_call_timeout, call_total, Duration::from_secs(13_000)),
        Some(Duration::from_secs(1400))
    );
    // Never negative; a spent total renders as zero, not as a fresh dispatch.
    assert_eq!(
        effective_call_budget(host_call_timeout, call_total, Duration::from_secs(20_000)),
        Some(Duration::ZERO)
    );
    // A host with no per-dispatch timeout falls back to the total, and vice
    // versa; neither means unbounded, as before.
    assert_eq!(effective_call_budget(None, call_total, Duration::ZERO), call_total);
    assert_eq!(effective_call_budget(host_call_timeout, None, Duration::ZERO), host_call_timeout);
    assert_eq!(effective_call_budget(None, None, Duration::ZERO), None);
}

/// A mid-attempt restart names the work as the agent's own; a new attempt
/// still says an earlier attempt left it.
#[test]
fn the_restart_preamble_says_the_workspace_is_as_the_agent_left_it() {
    let partial = PartialWork {
        patch_path: "p".into(),
        files: vec!["a.rs".into(), "b.rs".into()],
        bytes: 1,
        baseline_commit: "c".into(),
    };
    let restarted = with_restart_preamble("do the task", None, Some(&partial));
    assert!(restarted.starts_with(
        "This is the same attempt, restarted after the model connection ended; the workspace is exactly as you left it. Its uncommitted work (2 file(s)) has been applied to this workspace: a.rs, b.rs."
    ), "{restarted}");
    assert!(!restarted.contains("A previous attempt"));
    let resumed = with_host_preamble("do the task", None, Some(&partial));
    assert!(resumed.starts_with("A previous attempt at this task ran out of time"));
    assert!(!resumed.contains("same attempt"));
    // No partial, no sentence about one — a restart with a clean worktree
    // gets only the budget.
    assert_eq!(with_restart_preamble("do the task", None, None), "do the task");
}
