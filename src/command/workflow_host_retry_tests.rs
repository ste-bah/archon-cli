use super::*;
use std::cell::Cell;

#[tokio::test(start_paused = true)]
async fn host_retry_records_every_transport_failure_with_redacted_cause_and_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(archon_workflow::WorkflowSpec::from_yaml("schema: archon.workflow.v1\nname: retry-test\ntask: retry-test\nstages:\n  - id: stage-a\n    kind: agent\n").unwrap()).unwrap();
    let attempts = Cell::new(0u64);
    let secret = "sk-ant-abcdefghijklmnopqrstuvwxyz0123456789";
    let value = retry_waiting(
        "test-capability",
        || {
            attempts.set(attempts.get() + 1);
            std::future::ready(if attempts.get() <= 7 {
                Err(anyhow::Error::new(
                    archon_llm::provider::LlmError::Transport(
                        format!("TLS handshake unavailable: {secret}").into(),
                    ),
                ))
            } else {
                Ok(42)
            })
        },
        |attempt, delay, error| {
            record_retry(&store, &run.id, "test-capability", attempt, delay, error)
        },
        |delay| async move {
            tokio::time::sleep(delay).await;
            Ok(())
        },
    )
    .await
    .unwrap();
    assert_eq!(value, 42);
    assert_eq!(attempts.get(), 8);
    let raw = std::fs::read_to_string(store.events_path(&run.id)).unwrap();
    assert!(!raw.contains(secret), "{raw}");
    let events: Vec<serde_json::Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let retries: Vec<_> = events
        .iter()
        .filter(|event| event["detail"]["event"] == "host_capability_retry")
        .collect();
    assert_eq!(retries.len(), 7);
    for (index, event) in retries.iter().enumerate() {
        let detail = &event["detail"];
        assert_eq!(detail["attempt"], index + 1);
        assert_eq!(detail["next_attempt"], index + 2);
        assert_eq!(detail["capability"], "test-capability");
        assert!(
            detail["cause"]
                .as_str()
                .unwrap()
                .contains("TLS handshake unavailable")
        );
        assert!(detail["cause"].as_str().unwrap().contains("REDACTED"));
        assert_eq!(
            detail["next_delay_ms"],
            archon_workflow::llm_retry::retry_delay(index + 1).as_millis() as u64
        );
    }
}

#[tokio::test]
async fn host_retry_aborts_if_observable_run_event_cannot_be_written() {
    let attempts = Cell::new(0);
    let error = retry_waiting::<(), _, _, _, _, _>(
        "test-capability",
        || {
            attempts.set(attempts.get() + 1);
            std::future::ready(Err(super::super::workflow_host_fault::transient(
                "network unavailable".to_string(),
            )))
        },
        |_, _, _| anyhow::bail!("run event write refused"),
        |_| async { panic!("no silent wait") },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("run event write refused"));
    assert_eq!(attempts.get(), 1);
}

#[tokio::test(start_paused = true)]
async fn host_retry_pause_and_cancel_interrupt_capped_backoff_within_one_control_poll() {
    for cancel in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let mut run = store.create_run(archon_workflow::WorkflowSpec::from_yaml(
            "schema: archon.workflow.v1\nname: controlled-retry\ntask: retry\nstages:\n  - id: stage-a\n    kind: agent\n",
        ).unwrap()).unwrap();
        run.status = archon_workflow::RunStatus::Running;
        store.save_state(&run).unwrap();
        let attempts = Cell::new(0);
        let started = tokio::sync::Notify::new();
        let work = retry::<(), _, _>(
            "test-capability",
            (&store, &run.id, run.generation, None),
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 6 {
                    started.notify_one();
                }
                std::future::ready(Err(anyhow::Error::new(
                    archon_llm::provider::LlmError::Transport("DNS unavailable".into()),
                )))
            },
        );
        tokio::pin!(work);
        tokio::select! {
            result = &mut work => panic!("retry escaped before control: {result:?}"),
            _ = started.notified() => {},
        }
        archon_workflow::LifecycleController::new(store.clone())
            .apply(
                &run.id,
                if cancel {
                    archon_workflow::LifecycleAction::Cancel
                } else {
                    archon_workflow::LifecycleAction::Pause
                },
            )
            .unwrap();
        let before = tokio::time::Instant::now();
        let error = tokio::time::timeout(Duration::from_millis(101), work)
            .await
            .expect("control must interrupt within one poll")
            .unwrap_err();
        assert!(before.elapsed() <= Duration::from_millis(100));
        assert_eq!(attempts.get(), 6);
        let error = error.downcast::<archon_workflow::WorkflowError>().unwrap();
        assert!(matches!(
            (&error, cancel),
            (archon_workflow::WorkflowError::ControlCancelled(_), true)
                | (archon_workflow::WorkflowError::ControlPaused(_), false)
        ));
    }
}
