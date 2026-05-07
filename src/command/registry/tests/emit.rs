use super::*;
use archon_tui::app::TuiEvent;

fn make_emit_test_ctx(tui_tx: archon_tui::event_channel::TuiEventSender) -> CommandContext {
    CommandContext {
        tui_tx,
        status_snapshot: None,
        model_snapshot: None,
        cost_snapshot: None,
        mcp_snapshot: None,
        context_snapshot: None,
        session_id: None,
        memory: None,
        garden_config: None,
        fast_mode_shared: None,
        show_thinking: None,
        working_dir: None,
        skill_registry: None,
        denial_snapshot: None,
        effort_snapshot: None,
        permissions_snapshot: None,
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

/// Happy path — emit pushes the event into the channel and a
/// subsequent `try_recv` observes it byte-equivalent.
#[test]
fn emit_happy_path_delivers_event() {
    let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let ctx = make_emit_test_ctx(tx);

    ctx.emit(TuiEvent::TextDelta("hello".to_string()));

    match rx.try_recv() {
        Ok(TuiEvent::TextDelta(s)) => assert_eq!(s, "hello"),
        other => panic!("expected Ok(TextDelta(\"hello\")), got {other:?}"),
    }
}

// TASK-SESSION-LOOP-EXTRACT (A-2): the former
// `emit_full_channel_warns_and_does_not_panic` test has been
// DELETED. Unbounded channels cannot become full, so the "Full"
// branch the test exercised no longer exists in `emit`'s match
// body. The shipped silent-drop semantics that test guarded
// remain for the Closed branch (covered below) — no observable
// behavior change for a channel under realistic production load.

/// Closed-channel branch — drop the receiver before calling emit.
/// Must not panic; event is silently dropped (tracing::error! only).
#[test]
fn emit_closed_channel_errors_and_does_not_panic() {
    let (tx, rx) = archon_tui::event_channel::bounded_tui_event_channel();
    drop(rx);
    let ctx = make_emit_test_ctx(tx);

    // Receiver is gone — send returns Err. emit must not
    // panic and must not propagate the error.
    ctx.emit(TuiEvent::TextDelta("orphaned".to_string()));
}
