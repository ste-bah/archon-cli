//! An owner exemption never substitutes for the host's failure identity.
use super::*;
use crate::v2::verification::baseline_rule::OtherOwnerTest;
use crate::write_coordinator::worktree_isolation::run_git;

#[tokio::test]
async fn baseline_other_owner_claim_requires_every_named_host_signature() {
    let _clock = crate::v2::write::test_baseline_run_base::tests::freeze_runner_clock();
    let w = setup(false);
    // The real harness uses a custom panic hook that keeps the actual name,
    // location and payload, without emitting a process-specific thread number.
    let source = r#"
#[test]
fn same_id() {
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().unwrap();
        let thread = std::thread::current();
        let text = info.payload().downcast_ref::<String>().map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied()).unwrap();
        eprintln!("thread '{}' panicked at {}:{}:{}:\n{}",
            thread.name().unwrap_or("unnamed"), location.file(), location.line(), location.column(), text);
    }));
    let message = std::fs::read_to_string("failure.txt").unwrap();
    panic!("{}", message.trim());
}
"#;
    std::fs::write(w.repo.join("src/shared_tests.rs"), source).unwrap();
    std::fs::write(w.repo.join("failure.txt"), "first value\nold detail").unwrap();
    let runner = w._temp.path().join("bin/real-test-runner");
    let compiled = std::process::Command::new("rustc")
        .current_dir(&w.repo)
        .args([
            "--edition=2024",
            "--test",
            "-Dwarnings",
            "src/shared_tests.rs",
            "-o",
        ])
        .arg(&runner)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    for args in [
        &["add", "."][..],
        &["commit", "-qm", "recorded base failure"][..],
    ] {
        run_git(args, &w.repo).unwrap();
    }
    let base = head(&w.repo);
    bind_run(&w.store, &base);
    let command = format!("'{}' --exact same_id", runner.display());
    let mut universe = universe(&["src/mine.rs"]);
    universe.tasks[0].focused_tests = vec![format!("`{command}`")];
    universe.tasks[0].required_tools = vec![runner.to_string_lossy().into_owned()];
    universe.tasks.push(WorkflowV2TaskUniverseTask {
        canonical_task_id: "TASK-B".into(),
        files_expected_to_change: vec!["src/shared_tests.rs".into()],
        ..Default::default()
    });
    let path = std::env::join_paths([runner.parent().unwrap()]).unwrap();
    for task in &mut universe.tasks {
        task.freeze_focused_commands(&w.repo, &path);
    }
    assert!(universe.command_decisions_are_recorded());
    let mut stamp = stamp();
    stamp.base_commit = base;
    stamp.declared_commands = vec![command.clone()];
    stamp.other_owner = vec![OtherOwnerTest {
        test_id: "same_id".into(),
        owner_task: "TASK-B".into(),
    }];
    let by_item = BTreeMap::from([(ITEM.to_string(), stamp)]);
    for (case, message, changed) in [
        ("equal", None, false),
        ("spacing", Some("first  value\nold detail"), true),
        ("shortened-message", Some("first value"), true),
        ("disjoint", Some("second value"), true),
    ] {
        if let Some(message) = message {
            std::fs::write(w.repo.join("failure.txt"), message).unwrap();
            for args in [
                &["add", "."][..],
                &["commit", "-qm", "changed failure payload"][..],
            ] {
                run_git(args, &w.repo).unwrap();
            }
        }
        for other_owner in [false, true] {
            let mut by_item = by_item.clone();
            if !other_owner {
                by_item.get_mut(ITEM).unwrap().other_owner.clear();
            }
            let mut outcomes = vec![verdict(&["same_id"], vec![])];
            let report = outcomes[0].result.as_mut().unwrap();
            report.commands_run[0].command = command.clone();
            report.commands_run[0].output_summary.clear();
            let call = call(&[]);
            let tip = head(&w.repo);
            let proof = excuse_run_base_red_tests(
                &RunBaseRedContext {
                    store: &w.store,
                    dispatch: &w.host,
                    universe: Some(&universe),
                    repository_root: &w.repo,
                    call: &call,
                    judged_commit: Some(&tip),
                },
                &outcomes,
                &by_item,
                &BTreeMap::new(),
            )
            .await;
            enforce_baseline_tests_excusing(&mut outcomes, &by_item, &proof);
            assert_eq!(
                outcomes[0].status,
                if changed {
                    WorkflowV2Status::NeedsReview
                } else {
                    WorkflowV2Status::Accepted
                },
                "case={case}, other_owner={other_owner}: {proof:?}"
            );
        }
    }
}
