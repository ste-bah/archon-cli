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
    let partial = capture_partial_work(&first, &run_root, "agents-2", "agents-2-0", &["TASK-001".to_string()], None)
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
        capture_partial_work(&first, &temp.path().join("run"), "s", "i", &[], None)
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
        origin: None,
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
    let partial = capture_partial_work(&first, &temp.path().join("run"), "s", "i", &["TASK-001".to_string()], None)
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
    let plain = with_host_preamble("do the task", None, None, &Default::default());
    assert_eq!(plain, "do the task");
    let budgeted = with_host_preamble(
        "do the task",
        Some(std::time::Duration::from_secs(5400)),
        None,
        &Default::default(),
    );
    assert!(budgeted.starts_with("Time budget: this call has 90 minutes"));
    assert!(budgeted.contains("Write the deliverable files first"));
    assert!(budgeted.ends_with("\n\ndo the task"));
    let partial = PartialWork {
        patch_path: "p".into(),
        files: vec!["a.rs".into()],
        bytes: 1,
        baseline_commit: "c".into(),
        origin: None,
    };
    let both = with_host_preamble(
        "do the task",
        Some(std::time::Duration::from_secs(61)),
        Some(&partial),
        &Default::default(),
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
    let text = with_host_preamble("do the task", effective, None, &Default::default());
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
        origin: None,
    };
    let restarted = with_restart_preamble("do the task", None, Some(&partial), &Default::default());
    assert!(restarted.starts_with(
        "This is the same attempt, restarted after the model connection ended; the workspace is exactly as you left it. Its uncommitted work (2 file(s)) has been applied to this workspace: a.rs, b.rs."
    ), "{restarted}");
    assert!(!restarted.contains("A previous attempt"));
    let resumed = with_host_preamble("do the task", None, Some(&partial), &Default::default());
    assert!(resumed.starts_with("A previous attempt at this task ran out of time"));
    assert!(!resumed.contains("same attempt"));
    // No partial, no sentence about one — a restart with a clean worktree
    // gets only the budget.
    assert_eq!(with_restart_preamble("do the task", None, None, &Default::default()), "do the task");
}

fn rejected_origin() -> PartialOrigin {
    PartialOrigin::from_result(&WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "repository audit rejected unexplained or unauthorized changes: tests/extra.rs.".into(),
        residual_gaps: vec![
            crate::v2::WorkflowV2ResidualGap {
                id: "audit_unexplained_change".into(),
                description: "tests/extra.rs changed without an audit disposition".into(),
                severity: Some("blocker".into()),
            },
            crate::v2::WorkflowV2ResidualGap {
                id: "coverage_missing".into(),
                description: "no test covers the new branch".into(),
                severity: None,
            },
        ],
        ..Default::default()
    })
}

/// Issue-20, the live failure: a coder rejected by a gate was re-dispatched
/// with "ran out of time" and repeated the identical omission. The resume
/// preamble must carry the verdict: status, summary, and every gap.
#[test]
fn a_rejection_origin_names_the_status_summary_and_every_gap() {
    let partial = PartialWork {
        patch_path: "p".into(),
        files: vec!["a.rs".into()],
        bytes: 1,
        baseline_commit: "c".into(),
        origin: Some(rejected_origin()),
    };
    let text = with_host_preamble("do the task", None, Some(&partial), &Default::default());
    assert!(
        text.starts_with(
            "A previous attempt at this task was not accepted (status: needs_review): repository audit rejected unexplained or unauthorized changes: tests/extra.rs.\nIts unresolved gaps, which you must fix:\n- [blocker] audit_unexplained_change: tests/extra.rs changed without an audit disposition\n- [gap] coverage_missing: no test covers the new branch\nIts uncommitted work (1 file(s)) has been applied to this workspace: a.rs. Continue from that work; do not start over, and do not discard it unless it is wrong."
        ),
        "{text}"
    );
    assert!(!text.contains("ran out of time"));
    assert!(text.ends_with("\n\ndo the task"));
    // A restart in the same worktree keeps its own opening but is still told
    // the gaps the verdict named.
    let restarted = with_restart_preamble("do the task", None, Some(&partial), &Default::default());
    assert!(restarted.starts_with("This is the same attempt, restarted"), "{restarted}");
    assert!(restarted.contains("- [blocker] audit_unexplained_change:"));
    assert!(!restarted.contains("was not accepted"));
}

/// A host cut is not a verdict: the sentence stays, and the timeout gap the
/// interruption result carries is not presented as something to fix.
#[test]
fn a_timeout_origin_keeps_the_ran_out_of_time_sentence() {
    let origin = PartialOrigin::from_result(&super::super::errors::write_branch_interrupted_result(
        "agents-2-0",
        &serde_json::json!({"item": {"canonical_task_ids": ["TASK-001"]}}),
        "host call timed out after 7200s",
    ));
    assert!(origin.is_timeout(), "{origin:?}");
    assert!(!rejected_origin().is_timeout());
    // An agent's own verdict that mentions a timeout is still a verdict.
    let judged = PartialOrigin::from_result(&WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: "integration tests timed out; the retry loop never terminates".into(),
        ..Default::default()
    });
    assert!(!judged.is_timeout(), "{judged:?}");
    let partial = PartialWork {
        patch_path: "p".into(),
        files: vec!["a.rs".into()],
        bytes: 1,
        baseline_commit: "c".into(),
        origin: Some(origin),
    };
    let text = with_host_preamble("do the task", None, Some(&partial), &Default::default());
    assert!(text.starts_with("A previous attempt at this task ran out of time before finishing. Its uncommitted work (1 file(s))"), "{text}");
    assert!(!text.contains("unresolved gaps") && !text.contains("write_branch_timeout"));
}

/// The sidecar beside the patch flattens the partial, so the origin rides
/// along and comes back whole; a sidecar an older binary wrote has none and
/// still deserializes.
#[test]
fn the_sidecar_round_trips_the_origin_and_reads_without_one() {
    let temp = tempfile::tempdir().unwrap();
    let (_canonical, first, _second) = repo_with_worktrees(temp.path());
    std::fs::write(first.join("lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let run_root = temp.path().join("run");
    let partial = capture_partial_work(&first, &run_root, "agents-2", "agents-2-0", &["TASK-001".to_string()], Some(rejected_origin()))
        .unwrap()
        .unwrap();
    let sidecar_path = super::super::partial_work_lookup::sidecar_path(&partial.patch_path);
    let raw = std::fs::read_to_string(&sidecar_path).unwrap();
    assert!(raw.contains("\"origin\"") && raw.contains("audit_unexplained_change"), "{raw}");
    let sidecar: super::super::partial_work_lookup::PartialSidecar = serde_json::from_str(&raw).unwrap();
    assert_eq!(sidecar.partial, partial);
    assert_eq!(sidecar.partial.origin, Some(rejected_origin()));
    let store = WorkflowV2ResultStore::new(run_root.join("v2"));
    let found = latest_partial_for_tasks(&store, &["TASK-001".to_string()]).expect("found via sidecar");
    assert_eq!(found.origin, Some(rejected_origin()));
    // The pre-Issue-20 shape: no `origin` key at all.
    let legacy = format!(
        r#"{{"schema_version":1,"stage_id":"s","branch_id":"i","canonical_task_ids":["TASK-001"],"captured_at":"2026-09-15T00:00:00Z","patch_path":{},"files":["lib.rs"],"bytes":3,"baseline_commit":"c"}}"#,
        serde_json::to_string(&partial.patch_path).unwrap()
    );
    let parsed: super::super::partial_work_lookup::PartialSidecar = serde_json::from_str(&legacy).unwrap();
    assert_eq!(parsed.partial.origin, None);
    assert_eq!(parsed.partial.files, vec!["lib.rs".to_string()]);
    let legacy_partial: PartialWork = serde_json::from_str(r#"{"patch_path":"p","files":[],"bytes":0,"baseline_commit":"c"}"#).unwrap();
    assert_eq!(legacy_partial.origin, None);
}

/// A summary or gap description of any length is bounded in the origin, and
/// no more than `MAX_GAPS` gaps are kept.
#[test]
fn the_origin_caps_its_summary_and_gaps() {
    let result = WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: "s".repeat(5000),
        residual_gaps: (0..30)
            .map(|i| crate::v2::WorkflowV2ResidualGap {
                id: format!("g{i}"),
                description: "d".repeat(3000),
                severity: None,
            })
            .collect(),
        ..Default::default()
    };
    let origin = PartialOrigin::from_result(&result);
    assert_eq!(origin.status, "failed");
    assert_eq!(origin.summary.chars().count(), partial_origin::MAX_SUMMARY_CHARS + 3);
    assert_eq!(origin.residual_gaps.len(), partial_origin::MAX_GAPS);
    assert_eq!(origin.residual_gaps[0].description.chars().count(), partial_origin::MAX_GAP_DESCRIPTION_CHARS + 3);
}
