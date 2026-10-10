use super::{Scripted, ScriptedProcess, WorkflowError, WorkflowHostCommandExecutor, fixture, lint};

#[tokio::test]
async fn resumed_execution_of_the_same_call_uses_a_new_spill_tree() {
    let fixture = fixture(vec![
        Scripted::TimedOut(None),
        Scripted::TimedOut(None),
        Scripted::Exit(1),
    ]);
    let first = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await;
    assert!(matches!(first, Err(WorkflowError::ControlPaused(_))));
    let resumed = archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    let result = fixture
        .executor
        .execute(lint(), Some(resumed.generation))
        .await;
    assert!(result.is_ok(), "{result:?}");

    let spill_dirs = fixture.process.spill_dirs.lock().unwrap();
    assert_eq!(spill_dirs.len(), 3);
    assert_ne!(spill_dirs[0], spill_dirs[2]);
    for index in [0, 2] {
        assert!(
            spill_dirs[index]
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("attempt-1-")
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
