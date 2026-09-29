use super::*;
use crate::v2::agent_adapter::{
    WorkflowV2AgentAdapter, WorkflowV2AgentError, WorkflowV2AgentRequest,
};
use crate::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};

fn git(repo: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// `old` (the check fails there), `head` (the change under review).
fn repo() -> (tempfile::TempDir, String, String) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "t"]);
    std::fs::write(root.join("spec.json"), "{\"refs\": [\"pending\"]}\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "old"]);
    let old = git(root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("spec.json"), "{\"refs\": [\"registered\"]}\n").unwrap();
    git(root, &["commit", "-qam", "fix"]);
    let head = git(root, &["rev-parse", "HEAD"]);
    (temp, old, head)
}

fn failed(command: &str) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: command.to_string(),
        status: WorkflowV2CommandStatus::Failed,
        exit_code: Some(1),
        output_summary: "AssertionError: unregistered ref pending".to_string(),
        pre_existing: false,
    }
}

fn archive_demo(old: &str) -> String {
    format!(
        "mkdir -p /tmp/wt && git archive {old} | tar -x -C /tmp/wt && cp -R /srv/project/data /tmp/wt/ && cd /tmp/wt && python3 check.py"
    )
}

fn succeeded(command: &str) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "PASS".to_string(),
        ..failed(command)
    }
}

#[test]
fn only_the_narrow_demonstration_shape_is_read() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path();
    let r = repo.display();
    let accepted = [
        (archive_demo("abc1234"), "abc1234", "/tmp/wt", "python3 check.py"),
        (
            format!("git -C {r} archive --format=tar abc1234 src | tar -xf - -C /tmp/x && cd /tmp/x && make check && ls"),
            "abc1234", "/tmp/x", "make check && ls",
        ),
        (
            "git archive abc1234 | tar -xC /tmp/x && cd /tmp/x/ && python3 'frozen <check> body' | tail -3".into(),
            "abc1234", "/tmp/x", "python3 frozen <check> body | tail -3",
        ),
    ];
    for (command, rev, dir, check) in accepted {
        let found = shape(&command, repo).unwrap_or_else(|| panic!("{command}"));
        assert_eq!(
            (found.rev.as_str(), found.dir.as_str(), found.check.as_str()),
            (rev, dir, check),
            "{command}"
        );
    }
    let rejected = [
        // The check runs in the checkout under review.
        "git archive abc1234 | tar -x -C /tmp/wt && python3 check.py".to_string(),
        "cargo test; git archive abc1234 | tar -x -C /tmp/wt && cd /tmp/wt && true".into(),
        "git archive abc1234 | tar -x -C /tmp/wt && cargo test && cd /tmp/wt && true".into(),
        "git archive abc1234 | tar -x -C /tmp/wt && cd /tmp/wt && cd /repo && cargo test".into(),
        format!("git archive abc1234 | tar -x -C /tmp/o && cd /tmp/o && cargo test --manifest-path {r}/Cargo.toml"),
        "git archive abc1234 | tar -x -C /tmp/o && cd /tmp/o && make -C /repo check".into(),
        "git archive abc1234 | tar -x -C /tmp/o && cd /tmp/o && sh -c 'cd /repo && cargo test'".into(),
        "git archive abc1234 | tar -x -C /tmp/o && cd /tmp/o && pushd /repo && cargo test".into(),
        "git archive abc1234 | tar -x -C /tmp/o && cd /tmp/o && cat ../../repo/spec.json".into(),
        "git archive abc1234 | tar -x -C /tmp/o && cd /tmp/o && `cd /repo` && cargo test".into(),
        "D=/tmp/o; git archive abc1234 | tar -x -C $D && cd $D && cargo test".into(),
        // A directory that is not a clean one of its own.
        "git archive abc1234 | tar -x -C /tmp/o/../../repo && cd /tmp/o/../../repo && cargo test".into(),
        format!("git archive abc1234 | tar -x -C {r}/sub && cd {r}/sub && cargo test"),
        "git archive abc1234 | tar -x -C . && cd . && cargo test".into(),
        // Two materializations, or one the host cannot read.
        "git archive abc1234 >/dev/null; git archive HEAD | tar -x -C /tmp/o && cd /tmp/o && cargo test".into(),
        "git archive abc1234 | gzip | tar -xz -C /tmp/o && cd /tmp/o && cargo test".into(),
        "git -C /elsewhere archive abc1234 | tar -x -C /tmp/o && cd /tmp/o && cargo test".into(),
        "git show abc1234:spec.json | python3 check.py".into(),
        "git worktree add /tmp/w abc1234 && cd /tmp/w && cargo test".into(),
        "cargo test -p x".into(),
        // Preparation that carries the checkout's files into the old tree.
        "git archive abc1234 | tar -x -C /tmp/o && cp -R src /tmp/o/ && cd /tmp/o && cargo test".into(),
        format!("git archive abc1234 | tar -x -C /tmp/o && cp -R {r}/src /tmp/o/ && cd /tmp/o && cargo test"),
        "git archive abc1234 | tar -x -C /tmp/o && cd /tmp/o && cat ~/repo/spec.json".into(),
    ];
    for command in rejected {
        assert_eq!(shape(&command, repo), None, "{command}");
    }
}

#[test]
fn only_a_strictly_older_commit_with_another_tree_is_a_baseline() {
    let (temp, old, head) = repo();
    let root = temp.path();
    assert_eq!(older_commit(root, &old, "HEAD"), Some(old.clone()));
    assert_eq!(older_commit(root, &old[..9], &head), Some(old.clone()));
    assert_eq!(older_commit(root, "HEAD~1", "HEAD"), Some(old.clone()));
    // The change itself, by any name.
    assert_eq!(older_commit(root, "HEAD", "HEAD"), None);
    assert_eq!(older_commit(root, &head, "HEAD"), None);
    // Unknown, option-shaped, or not an ancestor.
    assert_eq!(older_commit(root, "deadbeef1234", "HEAD"), None);
    assert_eq!(older_commit(root, "--all", "HEAD"), None);
    git(root, &["checkout", "-qb", "side", &old]);
    std::fs::write(root.join("other.txt"), "x\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "side"]);
    let side = git(root, &["rev-parse", "HEAD"]);
    assert_eq!(older_commit(root, &side, &head), None);
    // An ancestor holding the very tree under review is the change.
    git(root, &["checkout", "-q", &head]);
    git(root, &["commit", "-q", "--allow-empty", "-m", "empty"]);
    assert_eq!(older_commit(root, &head, "HEAD"), None);
}

#[test]
fn classify_records_verified_demonstrations_and_drops_a_forged_list() {
    let (temp, old, _) = repo();
    let root = temp.path().to_string_lossy().into_owned();
    let mut result = crate::WorkflowV2Result {
        commands_run: vec![
            failed(&archive_demo(&old)),
            failed("python3 check.py --strict"),
            succeeded("python3 check.py"),
        ],
        data: json!({ BASELINE_DEMONSTRATIONS_KEY: [{ "command": "python3 check.py" }] }),
        ..Default::default()
    };
    classify(&mut result, Some(root.as_str()), None);
    let demo = result.commands_run[0].clone();
    let here = result.commands_run[1].clone();
    assert!(
        is_baseline_demonstration(&result, &demo),
        "{:#}",
        result.data
    );
    assert!(
        !is_baseline_demonstration(&result, &here),
        "{:#}",
        result.data
    );
    // Without its pass-on-new twin the old failure is the change's.
    result.commands_run.pop();
    classify(&mut result, Some(root.as_str()), None);
    assert!(
        !is_baseline_demonstration(&result, &demo),
        "{:#}",
        result.data
    );
    // No repository: nothing is a demonstration, and the forged list is gone.
    classify(&mut result, None, None);
    assert!(!is_baseline_demonstration(&result, &demo));
    assert!(result.data.get(BASELINE_DEMONSTRATIONS_KEY).is_none());
}

fn request(repository_root: &str) -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "verification-wave-review-verify-task-1".to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        role: "verifier".to_string(),
        task: "Verify the remediation.".to_string(),
        constraints: Vec::new(),
        input: json!({ "item": { "canonical_task_ids": ["TASK-001"] } }),
        repository_root: Some(repository_root.to_string()),
        project_artifacts: Default::default(),
        target_files: Vec::new(),
        target_ownership_scopes: Vec::new(),
    }
}

fn verdict(failed_command: &str) -> String {
    json!({
        "status": "accepted",
        "summary": "the check fails at the old commit and passes at the change",
        "files_changed": [],
        "commands_run": [
            { "kind": "test", "command": failed_command, "status": "failed", "exit_code": 1,
              "output_summary": "AssertionError: unregistered ref pending (at the pre-change commit)" },
            { "kind": "test", "command": "python3 check.py", "status": "succeeded", "exit_code": 0,
              "output_summary": "PASS" }
        ],
        "task_coverage": [{ "task_id": "TASK-001", "status": "accepted", "summary": "fixed",
            "evidence": [{ "kind": "test", "summary": "fail-on-old, pass-on-new" }] }]
    })
    .to_string()
}

/// Fails on 25ff60622: the demonstration was counted as a contradiction.
#[test]
fn a_fail_on_old_demonstration_does_not_contradict_an_accepted_verdict() {
    let (temp, old, _) = repo();
    let root = temp.path().to_string_lossy().into_owned();
    let result = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(&root), &verdict(&archive_demo(&old)))
        .expect("a failure at an older commit is evidence for the fix");
    assert_eq!(result.status, crate::WorkflowV2Status::Accepted);
    assert!(is_baseline_demonstration(&result, &result.commands_run[0]));
}

#[test]
fn a_failure_of_the_change_itself_still_contradicts_the_verdict() {
    let (temp, old, head) = repo();
    let root = temp.path().to_string_lossy().into_owned();
    for command in [
        // Against the commit under review, spelled as HEAD or by its sha.
        archive_demo("HEAD"),
        archive_demo(&head),
        // An old tree materialized, then the check run in the checkout.
        format!("git archive {old} | tar -x -C /tmp/wt && python3 check.py"),
        // No materialization at all.
        "python3 check.py --strict".to_string(),
    ] {
        let error = WorkflowV2AgentAdapter::new()
            .parse_agent_output(&request(&root), &verdict(&command))
            .expect_err("a failure of the change contradicts an accepted verdict");
        assert!(
            matches!(&error, WorkflowV2AgentError::AcceptedWithFailedTestCommands(c) if c == &[command.clone()]),
            "{command}: {error}"
        );
    }
}

#[test]
fn a_demonstration_without_its_pass_on_new_twin_still_contradicts() {
    let (temp, old, _) = repo();
    let root = temp.path().to_string_lossy().into_owned();
    let demo = archive_demo(&old);
    // Only the old failure and an unrelated success: the check was never
    // shown to pass on the change.
    let output = verdict(&demo).replace("\"python3 check.py\"", "\"git status\"");
    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request(&root), &output)
        .expect_err("no evidence the check passes on the change");
    assert!(
        matches!(&error, WorkflowV2AgentError::AcceptedWithFailedTestCommands(c) if c == &[demo.clone()]),
        "{error}"
    );
}

#[test]
fn only_the_exact_check_on_the_change_pairs_a_demonstration() {
    let (temp, old, head) = repo();
    let root = temp.path().to_string_lossy().into_owned();
    let demo = archive_demo(&old);
    let paired = |passing: &str| {
        let mut result = crate::WorkflowV2Result {
            commands_run: vec![failed(&demo), succeeded(passing)],
            ..Default::default()
        };
        classify(&mut result, Some(root.as_str()), None);
        is_baseline_demonstration(&result, &result.commands_run[0].clone())
    };
    assert!(paired("python3 check.py"));
    assert!(paired(&format!(
        "git archive {head} | tar -x -C /tmp/new && cd /tmp/new && python3 check.py"
    )));
    for loose in [
        "python3 check.py || true",
        "echo python3 check.py",
        "python3 check.py --other",
        "cd /tmp/wt && python3 check.py",
        demo.as_str(),
    ] {
        assert!(!paired(loose), "{loose}");
    }
}
