use super::*;

#[tokio::test(start_paused = true)]
async fn host_command_timeouts_without_progress_recover_to_one_returned_outcome() {
    assert_host_recovery(vec![Scripted::TimedOut(None), Scripted::TimedOut(None)], 2).await;
}

#[tokio::test(start_paused = true)]
async fn host_command_growing_then_unchanged_progress_recovers_without_pause() {
    assert_host_recovery(
        vec![
            Scripted::TimedOut(Some(1)),
            Scripted::TimedOut(Some(2)),
            Scripted::TimedOut(Some(3)),
            Scripted::TimedOut(Some(4)),
            Scripted::TimedOut(Some(4)),
        ],
        5,
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn host_command_unchanged_progress_recovers_without_operator_action() {
    assert_host_recovery(
        vec![Scripted::TimedOut(Some(4)), Scripted::TimedOut(Some(4))],
        2,
    )
    .await;
}

async fn assert_host_recovery(mut outcomes: Vec<Scripted>, failures: usize) {
    let expected_progress: Vec<_> = outcomes
        .iter()
        .map(|outcome| match outcome {
            Scripted::TimedOut(progress) => *progress,
            _ => panic!("expected timeout recovery fixture"),
        })
        .collect();
    outcomes.push(Scripted::Exit(1));
    let fixture = fixture(outcomes);
    let result = fixture
        .executor
        .execute(lint(), Some(fixture.generation))
        .await
        .unwrap();
    assert_eq!(result.exit_code, Some(1));
    assert!(!result.timed_out);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), failures + 1);
    let state = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(state.status, RunStatus::Running);
    assert_eq!(state.generation, fixture.generation);
    assert_eq!(state.stages["host-call"].status, StageStatus::Running);
    assert!(events_named(&events(&fixture), "host_command_operational_pause").is_empty());
    let stored = events(&fixture);
    let retries = events_named(&stored, "host_command_operational_retry");
    assert_eq!(retries.len(), failures, "{stored:?}");
    for (index, event) in retries.iter().enumerate() {
        assert_eq!(event["kind"], "stage_stalled");
        let detail = &event["detail"];
        assert_eq!(detail["attempt"], index + 1);
        assert_eq!(detail["reason"], "timed_out");
        assert!(detail["elapsed_secs"].is_u64());
        assert_eq!(
            detail["progress"],
            serde_json::json!(expected_progress[index])
        );
        assert_eq!(detail["command_id"], "task-set-lint");
        assert!(detail["limit_secs"].as_u64().unwrap() > 0);
        assert_eq!(detail["attempts"].as_array().unwrap().len(), 1);
    }
    let spills = fixture.process.spill_dirs.lock().unwrap();
    assert_eq!(spills.len(), failures + 1);
    for (index, directory) in spills.iter().enumerate() {
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&format!("attempt-{}-", index + 1))
        );
        assert_eq!(directory.parent(), spills[0].parent());
    }
    assert_eq!(
        spills
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        failures + 1
    );
}

#[tokio::test(start_paused = true)]
async fn host_command_zero_progress_timeouts_recover_without_operator_action() {
    assert_host_recovery(
        vec![Scripted::TimedOut(Some(0)), Scripted::TimedOut(Some(0))],
        2,
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn host_command_incomplete_resumable_retries_with_backoff_to_publication() {
    let candidate = CANDIDATE.as_bytes().to_vec();
    let fixture = fixture(vec![
        Scripted::Incomplete(2),
        Scripted::Incomplete(5),
        Scripted::Publish(candidate.clone()),
    ]);
    let task_file = fixture.context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"live-before").unwrap();
    seed_frozen_chain(&fixture.context, &task_file);
    let request = HostCommandRequest::new("land-task-body", Some(CANDIDATE.into())).unwrap();

    let started = tokio::time::Instant::now();
    let result = fixture
        .executor
        .execute(request, Some(fixture.generation))
        .await
        .unwrap();

    assert!(started.elapsed() >= std::time::Duration::from_millis(1500));
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 3);
    assert!(result.reusable(), "{result:?}");
    assert_eq!(std::fs::read(&task_file).unwrap(), candidate);
    let run = fixture.store.load_state(&fixture.run_id).unwrap();
    assert_eq!(run.status, RunStatus::Running);
    assert_eq!(run.generation, fixture.generation);
    assert!(events_named(&events(&fixture), "host_command_operational_pause").is_empty());
}

#[tokio::test]
async fn host_command_permanent_exit_and_stage_refusal_remain_single_attempt() {
    let fixture = fixture(vec![Scripted::Exit(1)]);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fixture.executor.execute(lint(), Some(fixture.generation)),
    )
    .await
    .expect("permanent exit must finish")
    .unwrap();
    assert_eq!(result.exit_code, Some(1));
    assert!(!result.reusable() && !result.timed_out);
    assert_eq!(fixture.process.calls.load(Ordering::SeqCst), 1);

    let fixture_b = self::fixture(vec![Scripted::Error(
        "host command 'task-set-lint' failed: permission denied",
    )]);
    let result_b = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fixture_b
            .executor
            .execute(lint(), Some(fixture_b.generation)),
    )
    .await
    .expect("permanent refusal must finish")
    .unwrap_err();
    let error = result_b;
    assert!(matches!(error, WorkflowError::StageFailed(_)), "{error:?}");
    assert!(error.to_string().contains("permission denied"), "{error:?}");
    assert_eq!(fixture_b.process.calls.load(Ordering::SeqCst), 1);

    for fixture in [&fixture, &fixture_b] {
        let run = fixture.store.load_state(&fixture.run_id).unwrap();
        assert_eq!(run.status, RunStatus::Running);
        assert_eq!(run.generation, fixture.generation);
        assert!(events(fixture).is_empty());
    }
}
