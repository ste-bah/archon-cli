use super::*;

#[test]
fn activity_event_carries_required_context() {
    let event = AgentActivityEvent::new(
        "session-1",
        AgentActivityKind::AgentSpawned,
        AgentActivityStatus::Running,
        "spawned explore",
    )
    .with_run_id("run-1")
    .with_parent_id("parent")
    .with_agent_id("agent-1")
    .with_subagent_id("sub-1")
    .with_agent_key("explore")
    .with_subagent_type("explore")
    .with_artifact_id("artifact-1")
    .with_provider_model("anthropic", "claude")
    .with_cost_usd(0.25)
    .touch();

    assert_eq!(event.session_id, "session-1");
    assert_eq!(event.kind, AgentActivityKind::AgentSpawned);
    assert_eq!(event.status, AgentActivityStatus::Running);
    assert_eq!(event.run_id.as_deref(), Some("run-1"));
    assert_eq!(event.parent_id.as_deref(), Some("parent"));
    assert_eq!(event.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(event.subagent_id.as_deref(), Some("sub-1"));
    assert_eq!(event.agent_key.as_deref(), Some("explore"));
    assert_eq!(event.subagent_type.as_deref(), Some("explore"));
    assert_eq!(event.artifact_id.as_deref(), Some("artifact-1"));
    assert_eq!(event.provider.as_deref(), Some("anthropic"));
    assert_eq!(event.model.as_deref(), Some("claude"));
    assert_eq!(event.cost_usd, Some(0.25));
    assert!(event.updated_at.is_some());
}

#[test]
fn in_memory_sink_preserves_event_order() {
    let sink = InMemoryActivitySink::new();
    sink.emit(AgentActivityEvent::new(
        "session-1",
        AgentActivityKind::ParentTurnStarted,
        AgentActivityStatus::Running,
        "turn started",
    ));
    sink.emit(AgentActivityEvent::new(
        "session-1",
        AgentActivityKind::ParentTurnCompleted,
        AgentActivityStatus::Completed,
        "turn complete",
    ));

    let events = sink.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, AgentActivityKind::ParentTurnStarted);
    assert_eq!(events[1].kind, AgentActivityKind::ParentTurnCompleted);
}

#[test]
fn sink_trait_object_records_events() {
    let sink = InMemoryActivitySink::new();
    let trait_object: &dyn AgentActivitySink = &sink;

    trait_object.emit(AgentActivityEvent::new(
        "session-1",
        AgentActivityKind::ToolStarted,
        AgentActivityStatus::Running,
        "Read started",
    ));

    assert_eq!(sink.len(), 1);
}

#[test]
fn jsonl_writer_opens_once_across_multiple_events() {
    use std::cell::Cell;
    use std::io::Cursor;

    let opens = Cell::new(0);
    let mut writer: Option<Cursor<Vec<u8>>> = None;

    for message in ["first", "second"] {
        append_activity_event_with_writer(
            &mut writer,
            AgentActivityEvent::new(
                "session-1",
                AgentActivityKind::ToolCompleted,
                AgentActivityStatus::Completed,
                message,
            ),
            || {
                opens.set(opens.get() + 1);
                Ok(Cursor::new(Vec::new()))
            },
        )
        .expect("append event");
    }

    assert_eq!(opens.get(), 1);
    let output = String::from_utf8(writer.unwrap().into_inner()).expect("utf8 jsonl");
    assert_eq!(output.lines().count(), 2);
}

#[test]
fn jsonl_writer_reopens_after_write_failure() {
    struct TestWriter {
        fail: bool,
    }

    impl Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fail {
                return Err(std::io::Error::other("synthetic write failure"));
            }
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut opens = 0;
    let mut writer = None;
    let mut append = |message| {
        append_activity_event_with_writer(
            &mut writer,
            AgentActivityEvent::new(
                "session-1",
                AgentActivityKind::ToolCompleted,
                AgentActivityStatus::Completed,
                message,
            ),
            || {
                opens += 1;
                Ok(TestWriter { fail: opens == 1 })
            },
        )
    };

    assert!(append("first").is_err());
    append("second").expect("reopen after failure");
    assert_eq!(opens, 2);
}

#[test]
fn jsonl_writer_reopens_after_flush_failure() {
    struct TestWriter {
        fail_flush: bool,
    }

    impl Write for TestWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            if self.fail_flush {
                return Err(std::io::Error::other("synthetic flush failure"));
            }
            Ok(())
        }
    }

    let mut opens = 0;
    let mut writer = None;
    let mut append = |message| {
        append_activity_event_with_writer(
            &mut writer,
            AgentActivityEvent::new(
                "session-1",
                AgentActivityKind::ToolCompleted,
                AgentActivityStatus::Completed,
                message,
            ),
            || {
                opens += 1;
                Ok(TestWriter {
                    fail_flush: opens == 1,
                })
            },
        )
    };

    assert!(append("first").is_err());
    append("second").expect("reopen after flush failure");
    assert_eq!(opens, 2);
}

#[test]
fn jsonl_sink_persists_events_for_restart_readback() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = activity_jsonl_path(dir.path(), "session-1");
    let sink = JsonlActivitySink::new(path.clone());

    sink.emit(AgentActivityEvent::new(
        "session-1",
        AgentActivityKind::AgentSpawned,
        AgentActivityStatus::Running,
        "spawned researcher",
    ));
    sink.emit(AgentActivityEvent::new(
        "session-1",
        AgentActivityKind::AgentCompleted,
        AgentActivityStatus::Completed,
        "completed researcher",
    ));

    let events = read_activity_jsonl(path).expect("read persisted events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, AgentActivityKind::AgentSpawned);
    assert_eq!(events[1].status, AgentActivityStatus::Completed);
}

#[test]
fn jsonl_sink_redacts_secret_shapes_before_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = activity_jsonl_path(dir.path(), "session-2");
    let sink = JsonlActivitySink::new(path.clone());
    let secret = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz";

    sink.emit(AgentActivityEvent::new(
        "session-2",
        AgentActivityKind::ToolFailed,
        AgentActivityStatus::Failed,
        format!("provider failed with {secret}"),
    ));

    let raw = std::fs::read_to_string(&path).expect("raw jsonl");
    assert!(!raw.contains(secret));
    assert!(raw.contains("***REDACTED***"));
    let events = read_activity_jsonl(path).expect("read persisted events");
    assert_eq!(events[0].message, "provider failed with ***REDACTED***");
}

#[test]
fn jsonl_reader_ignores_blank_lines() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = activity_jsonl_path(dir.path(), "session-3");
    std::fs::create_dir_all(path.parent().expect("activity dir")).expect("mkdir");
    let event = AgentActivityEvent::new(
        "session-3",
        AgentActivityKind::Cancelled,
        AgentActivityStatus::Cancelled,
        "cancelled",
    );
    std::fs::write(
        &path,
        format!("\n{}\n\n", serde_json::to_string(&event).expect("json")),
    )
    .expect("write jsonl");

    let events = read_activity_jsonl(path).expect("read persisted events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, AgentActivityKind::Cancelled);
}
