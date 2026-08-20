//! Event-handling coverage for `tui_events.rs`, split out for the 500-line
//! ceiling (#192). Declared with `#[path]` from `tui_events.rs`, so `super`
//! still means that module and the tests read exactly as they did in place.

use super::*;
use serial_test::serial;
use std::time::Duration;

// Both tests below clear and read the process-global task registry
// (archon_observability::reset_task_registry_for_tests + task_snapshots).
// task_registry.rs documents this race: parallel tests wipe each other's
// entries mid-flight. Marking them #[serial(task_registry)] matches the
// pattern the registry's own tests use (task_registry.rs:163,178,189).
// Surfaced by CI run 25541207525 on commit bee8d8b under cargo llvm-cov,
// where instrumentation widens the race window enough to flip pass→fail.
#[tokio::test]
#[serial(task_registry)]
async fn turn_complete_flushes_pending_input_without_blocking_when_channel_has_room() {
    archon_observability::reset_task_registry_for_tests();
    let mut app = App::new();
    app.pending_input.push("first".to_string());
    app.pending_input.push("second".to_string());
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);

    handle_tui_event(
        &mut app,
        TuiEvent::TurnComplete {
            input_tokens: 1,
            output_tokens: 1,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        },
        &tx,
    )
    .await;

    assert!(app.pending_input.is_empty());
    assert_eq!(rx.try_recv().unwrap(), "first");
    assert_eq!(rx.try_recv().unwrap(), "second");
}

#[tokio::test]
#[serial(task_registry)]
async fn turn_complete_does_not_block_when_input_channel_is_full() {
    archon_observability::reset_task_registry_for_tests();
    let mut app = App::new();
    app.pending_input.push("queued".to_string());
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    tx.try_send("occupied".to_string()).unwrap();

    tokio::time::timeout(
        Duration::from_millis(50),
        handle_tui_event(
            &mut app,
            TuiEvent::TurnComplete {
                input_tokens: 1,
                output_tokens: 1,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            &tx,
        ),
    )
    .await
    .expect("TurnComplete handler must not await on a full input channel");

    assert!(app.pending_input.is_empty());
    assert_eq!(rx.try_recv().unwrap(), "occupied");
    assert!(
        archon_observability::task_snapshots()
            .iter()
            .any(|task| task.name == "tui-pending-input-flush")
    );
    archon_observability::abort_alive_tasks();
}

#[tokio::test]
async fn zero_usage_turn_preserves_preflight_context_pressure() {
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);

    handle_tui_event(
        &mut app,
        TuiEvent::ContextPressureUpdated {
            tokens_used: 121_000,
            context_window: 1_050_000,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            context_name: Some("main".into()),
            resolution_source: Some("bundled-catalog".into()),
            heaviest_message_tokens: 42_000,
            top_contributors: vec![(12, 42_000), (3, 8_000)],
            attributed_total: 121_000,
        },
        &tx,
    )
    .await;
    handle_tui_event(
        &mut app,
        TuiEvent::TurnComplete {
            input_tokens: 0,
            output_tokens: 10,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        },
        &tx,
    )
    .await;

    assert_eq!(app.status.context_tokens_used, 121_000);
    assert_eq!(app.status.context_name.as_deref(), Some("main"));
}

#[tokio::test]
async fn thinking_toggle_message_does_not_finish_active_thinking() {
    let mut app = App::new();
    app.on_thinking_delta("retained before toggle");
    let (tx, _rx) = tokio::sync::mpsc::channel(1);

    handle_tui_event(&mut app, TuiEvent::ThinkingToggle(true), &tx).await;

    assert!(app.show_thinking);
    assert!(app.thinking.active);
    assert_eq!(app.thinking.accumulated, "retained before toggle");
    assert!(app.thinking_blocks.is_empty());
    assert!(
        app.output
            .all_lines()
            .iter()
            .any(|line| line.contains("Thinking display enabled."))
    );

    app.on_thinking_delta(" and after toggle");
    app.on_turn_complete();
    assert_eq!(app.thinking_blocks.len(), 1);
    assert_eq!(
        app.thinking_blocks[0].text,
        "retained before toggle and after toggle"
    );
}

#[tokio::test]
async fn open_thinking_archive_event_selects_the_latest_block() {
    let mut app = App::new();
    app.on_thinking_delta("first thought");
    app.on_turn_complete();
    app.on_thinking_delta("second thought");
    app.on_turn_complete();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);

    handle_tui_event(&mut app, TuiEvent::OpenThinkingArchive, &tx).await;

    assert_eq!(app.thinking_archive_selection(), Some(1));
}

#[tokio::test]
async fn ask_user_prompt_sets_modal_state_and_logs_question() {
    let mut app = App::new();
    app.ask_user_draft = "stale answer".into();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);

    handle_tui_event(
        &mut app,
        TuiEvent::AskUserPrompt {
            question: "Choose a path".into(),
            kind: archon_core::agent::AskUserPromptKind::Ordinary,
        },
        &tx,
    )
    .await;

    assert_eq!(app.ask_user_prompt.as_deref(), Some("Choose a path"));
    assert!(app.ask_user_draft.is_empty());
    assert!(
        app.output
            .all_lines()
            .iter()
            .any(|line| line.contains("[question] Choose a path"))
    );
}

#[tokio::test]
#[serial]
async fn resize_and_done_update_canonical_loop_state() {
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);

    handle_tui_event(
        &mut app,
        TuiEvent::Resize {
            cols: 200,
            rows: 60,
        },
        &tx,
    )
    .await;
    handle_tui_event(&mut app, TuiEvent::Done, &tx).await;

    assert_eq!(crate::layout::last_known_size(), (200, 60));
    assert!(app.should_quit);
}
