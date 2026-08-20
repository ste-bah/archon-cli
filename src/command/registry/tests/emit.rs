use super::*;
use archon_tui::app::TuiEvent;

fn make_emit_test_ctx(tui_tx: archon_tui::event_channel::TuiEventSender) -> CommandContext {
    CommandContext {
        tui_tx,
        pending_tui_events: std::sync::Mutex::new(Vec::new()),
        status_snapshot: None,
        model_snapshot: None,
        cost_snapshot: None,
        mcp_snapshot: None,
        context_snapshot: None,
        session_id: None,
        session_store: None,
        memory: None,
        default_model: None,
        garden_config: None,
        fast_mode_shared: None,
        show_thinking: None,
        working_dir: None,
        skill_registry: None,
        denial_snapshot: None,
        effort_snapshot: None,
        permissions_snapshot: None,
        feedback_snapshot: None,
        plan_snapshot: None,
        copy_snapshot: None,
        doctor_snapshot: None,
        usage_snapshot: None,
        config_path: None,
        auth_label: None,
        agent_registry: None,
        task_service: None,
        coding_pipeline: None,
        research_pipeline: None,
        llm_adapter: None,
        leann: None,
        pending_effect: None,
        pending_effort_set: None,
        pending_export: None,
        cozo_db: None,
        governed_learning_db: None,
        // Reference: archon-pipeline/src/learning/gnn/auto_trainer.rs.
        // Test fixture — emit() doesn't touch this field.
        auto_trainer: None,
        sandbox_flag: None,
        hook_registry: None,
        plugin_enable_state: None,
        cancel_handle: None,
        agent_dispatcher: None,
    }
}

/// Happy path — flushing makes the event byte-equivalent at the receiver.
#[tokio::test]
async fn emit_happy_path_delivers_event() {
    let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let ctx = make_emit_test_ctx(tx);

    ctx.emit(TuiEvent::TextDelta("hello".to_string()));
    ctx.flush_events().await.expect("flush event");

    match rx.try_recv() {
        Ok(TuiEvent::TextDelta(s)) => assert_eq!(s, "hello"),
        other => panic!("expected Ok(TextDelta(\"hello\")), got {other:?}"),
    }
}

#[test]
fn command_handlers_do_not_bypass_buffered_emission() {
    fn visit(path: &std::path::Path, offenders: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(path).expect("read command source directory") {
            let entry = entry.expect("read command source entry");
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|value| value.to_str()) != Some("tests") {
                    visit(&path, offenders);
                }
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                let source = std::fs::read_to_string(&path).expect("read command source");
                let source_without_line_comments = source
                    .lines()
                    .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
                    .collect::<String>();
                let compact: String = source_without_line_comments
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect();
                if compact.contains("ctx.tui_tx.send(") {
                    offenders.push(path);
                }
            }
        }
    }

    let mut offenders = Vec::new();
    visit(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/command"),
        &mut offenders,
    );
    assert!(
        offenders.is_empty(),
        "command handlers bypass buffered TUI emission: {offenders:?}"
    );
}

/// Full-channel branch — synchronous handlers buffer their event, then the
/// async dispatch seam waits for capacity and preserves order.
#[tokio::test]
async fn flush_events_waits_for_capacity_without_losing_event() {
    let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).expect("fill queue");
    let ctx = make_emit_test_ctx(tx);

    ctx.emit(TuiEvent::Error("preserved".to_string()));
    let flush = tokio::spawn(async move { ctx.flush_events().await });
    tokio::task::yield_now().await;
    assert!(!flush.is_finished(), "flush must await queue capacity");

    assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
    flush
        .await
        .expect("flush task")
        .expect("flush buffered event");
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::Error(text)) if text == "preserved"
    ));
}

#[tokio::test]
async fn flush_events_preserves_order_across_saturation_boundary() {
    let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(2);
    let ctx = make_emit_test_ctx(tx);

    ctx.emit(TuiEvent::GenerationStarted);
    ctx.emit(TuiEvent::Error("buffered-first".to_string()));
    ctx.emit(TuiEvent::SlashCommandComplete);

    let flush = tokio::spawn(async move { ctx.flush_events().await });
    assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::Error(text)) if text == "buffered-first"
    ));
    flush
        .await
        .expect("flush task")
        .expect("flush buffered event");
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::SlashCommandComplete)
    ));
}

#[tokio::test]
async fn flush_events_rejects_oversized_fallback_without_retaining_payload() {
    let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).expect("fill queue");
    let ctx = make_emit_test_ctx(tx);
    let oversized = "x".repeat(archon_tui::event_channel::MAX_COALESCED_CONTENT_BYTES + 1);

    ctx.emit(TuiEvent::TextDelta(oversized.clone()));
    {
        let pending = ctx.pending_tui_events.lock().expect("pending events");
        assert_eq!(pending.len(), 1);
        assert!(matches!(
            pending.first(),
            Some(TuiEvent::Error(message))
                if message.contains("bounded command event buffer")
                    && !message.contains(&oversized)
        ));
    }
    let flush = tokio::spawn(async move { ctx.flush_events().await });

    assert!(matches!(rx.recv().await, Some(TuiEvent::GenerationStarted)));
    flush
        .await
        .expect("flush task")
        .expect("flush bounded rejection");
    assert!(matches!(
        rx.recv().await,
        Some(TuiEvent::Error(message)) if message.contains("bounded command event buffer")
    ));
}

#[test]
fn emit_bounds_pending_event_count_under_saturation() {
    let (tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).expect("fill queue");
    let ctx = make_emit_test_ctx(tx);

    for index in 0..10_000 {
        ctx.emit(TuiEvent::Error(format!("event-{index}")));
    }

    let pending = ctx.pending_tui_events.lock().expect("pending events");
    assert!(
        pending.len() <= 64,
        "pending event count was {}",
        pending.len()
    );
    assert!(matches!(
        pending.last(),
        Some(TuiEvent::Error(message)) if message.contains("bounded command event buffer")
    ));
}

#[test]
fn emit_overflow_preserves_all_previously_accepted_events() {
    let (tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).expect("fill queue");
    let ctx = make_emit_test_ctx(tx);

    for index in 0..64 {
        ctx.emit(TuiEvent::Error(format!("event-{index}")));
    }

    let pending = ctx.pending_tui_events.lock().expect("pending events");
    assert_eq!(pending.len(), 64);
    for index in 0..63 {
        assert!(matches!(
            &pending[index],
            TuiEvent::Error(message) if message == &format!("event-{index}")
        ));
    }
    assert!(matches!(
        pending.last(),
        Some(TuiEvent::Error(message)) if message.contains("bounded command event buffer")
    ));
}

#[test]
fn emit_deduplicates_bounded_rejection_marker() {
    let (tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    tx.send(TuiEvent::GenerationStarted).expect("fill queue");
    let ctx = make_emit_test_ctx(tx);
    let oversized = "x".repeat(archon_tui::event_channel::MAX_COALESCED_CONTENT_BYTES + 1);

    ctx.emit(TuiEvent::TextDelta(oversized.clone()));
    ctx.emit(TuiEvent::Error("normal".into()));
    ctx.emit(TuiEvent::TextDelta(oversized));

    let pending = ctx.pending_tui_events.lock().expect("pending events");
    let rejection_count = pending
        .iter()
        .filter(|event| {
            matches!(event, TuiEvent::Error(message) if message.contains("bounded command event buffer"))
        })
        .count();
    assert_eq!(rejection_count, 1);
}

/// Closed-channel branch — flushing reports receiver shutdown.
#[tokio::test]
async fn flush_events_reports_closed_receiver() {
    let (tx, rx) = archon_tui::event_channel::bounded_tui_event_channel();
    drop(rx);
    let ctx = make_emit_test_ctx(tx);

    ctx.emit(TuiEvent::TextDelta("orphaned".to_string()));

    assert!(ctx.flush_events().await.is_err());
}
