use super::{Scripted, ScriptedProcess, WorkflowHostCommandExecutor, fixture, lint};

#[tokio::test(start_paused = true)]
async fn transient_retries_and_resumed_invocations_use_distinct_numbered_spill_trees() {
    let fixture = fixture(vec![
        Scripted::TimedOut(None),
        Scripted::TimedOut(None),
        Scripted::Exit(1),
    ]);
    fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap();
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Pause)
        .unwrap();
    let resumed = archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    fixture
        .executor
        .execute(lint(), Some(resumed.generation))
        .await
        .unwrap();
    let spills = fixture.process.spill_dirs.lock().unwrap();
    assert_eq!(spills.len(), 4);
    assert_eq!(
        spills
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4
    );
    for (index, number) in [1, 2, 3, 1].into_iter().enumerate() {
        assert!(
            spills[index]
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&format!("attempt-{number}-")),
            "{:?}",
            spills[index]
        );
    }
}

pub(super) fn assert_attempt_spill_dirs(process: &ScriptedProcess) {
    let spill_dirs = process.spill_dirs.lock().unwrap();
    assert_eq!(spill_dirs.len(), 2);
    assert_ne!(spill_dirs[0], spill_dirs[1], "retry reused spill files");
    assert!(
        spill_dirs[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("attempt-1-")
    );
    assert!(
        spill_dirs[1]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("attempt-2-")
    );
    assert_eq!(spill_dirs[0].parent(), spill_dirs[1].parent());
}

#[tokio::test(start_paused = true)]
async fn host_command_two_attempts_keep_separate_numbered_spills_before_completion() {
    let fixture = fixture(vec![Scripted::TimedOut(None), Scripted::Exit(1)]);
    fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap();
    assert_attempt_spill_dirs(&fixture.process);
}
