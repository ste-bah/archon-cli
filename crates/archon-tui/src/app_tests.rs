//! Unit tests for `App` state transitions (TUI-310 extraction).
//!
//! Moved from `app.rs` as part of the event-loop extraction so `app.rs`
//! can remain a thin orchestrator module (<500 lines). These tests
//! exercise `App` methods — no event-loop coverage here (that lives in
//! `tests/event_loop_smoke.rs` and `tests/app_run_e2e.rs`).

use super::{AgentActivityRole, App, EvidenceViewState, ViewId};
use crate::events::{AgentActivityStatus, AgentActivityUpdate};

#[test]
fn app_text_delta() {
    let mut app = App::new();
    app.on_text_delta("Hello ");
    app.on_text_delta("world");
    assert_eq!(app.output.all_lines(), vec!["Hello world"]);
}

#[test]
fn app_submit_input_does_not_set_generating() {
    let mut app = App::new();
    app.input.insert('t');
    app.input.insert('e');
    app.input.insert('s');
    app.input.insert('t');
    let text = app.submit_input();
    assert_eq!(text, "test");
    // submit_input never sets is_generating — that is done by
    // GenerationStarted event from main.rs before agent.process_message()
    assert!(!app.is_generating);
}

#[test]
fn app_tool_lifecycle() {
    let mut app = App::new();
    // GenerationStarted sets is_generating (not on_tool_start)
    app.on_generation_started();
    assert!(app.is_generating);
    app.on_tool_start("Read", "tool-123");
    assert_eq!(app.active_tool.as_deref(), Some("Read"));
    app.on_tool_complete("Read", "tool-123", true, "file contents here");
    assert!(app.active_tool.is_none());
    // Successful tool calls do NOT append to output (no noise)
    assert!(app.output.all_lines().is_empty());
    // But the tool output state is tracked
    assert_eq!(app.tool_outputs.len(), 1);
    assert_eq!(app.tool_outputs[0].tool_name, "Read");
    assert!(
        app.agent_activity
            .iter()
            .any(|row| row.id == "parent" && row.status == AgentActivityStatus::Running)
    );
}

#[test]
fn app_agent_tool_lifecycle_updates_subagent_activity_row() {
    let mut app = App::new();
    app.on_generation_started();
    app.on_tool_start("Agent", "agent-tool-1");
    assert!(app.agent_activity.iter().any(|row| {
        row.id == "agent-tool-1"
            && row.role == AgentActivityRole::Subagent
            && row.status == AgentActivityStatus::Running
    }));

    app.on_tool_complete("Agent", "agent-tool-1", true, "subagent result");
    assert!(
        app.agent_activity
            .iter()
            .any(|row| { row.id == "agent-tool-1" && row.status == AgentActivityStatus::Complete })
    );
}

#[test]
fn app_applies_external_agent_activity_update() {
    let mut app = App::new();
    app.on_agent_activity(AgentActivityUpdate {
        id: "bg-1".into(),
        name: "Background reviewer".into(),
        role: AgentActivityRole::Background,
        status: AgentActivityStatus::WaitingForTool,
        current_tool: Some("Read".into()),
        detail: Some("auditing".into()),
        run_id: Some("run-1".into()),
        parent_id: Some("parent".into()),
        artifact_id: Some("artifact-1".into()),
        provider: Some("openai-codex".into()),
        model: Some("gpt-5.4".into()),
        cost_usd: Some(0.02),
    });

    let row = app.agent_activity.first().expect("activity row");
    assert_eq!(row.id, "bg-1");
    assert_eq!(row.role, AgentActivityRole::Background);
    assert_eq!(row.current_tool.as_deref(), Some("Read"));
    assert_eq!(row.artifact_id.as_deref(), Some("artifact-1"));
    assert_eq!(row.provider.as_deref(), Some("openai-codex"));
    assert_eq!(row.model.as_deref(), Some("gpt-5.4"));
}

#[test]
fn app_accepts_canonical_activity_event_via_update_bridge() {
    let mut app = App::new();
    let update = AgentActivityUpdate::from(
        archon_observability::AgentActivityEvent::new(
            "session-1",
            archon_observability::AgentActivityKind::AgentSpawned,
            archon_observability::AgentActivityStatus::Running,
            "spawned explore",
        )
        .with_subagent_id("subagent-1")
        .with_agent_key("explore"),
    );

    app.on_agent_activity(update);

    let row = app.agent_activity.first().expect("activity row");
    assert_eq!(row.id, "subagent-1");
    assert_eq!(row.name, "explore");
    assert_eq!(row.role, AgentActivityRole::Subagent);
    assert_eq!(row.status, AgentActivityStatus::Running);
}

#[test]
fn app_maps_canonical_activity_statuses_without_collapsing_state() {
    let cases = [
        (
            archon_observability::AgentActivityStatus::Queued,
            AgentActivityStatus::Queued,
        ),
        (
            archon_observability::AgentActivityStatus::Waiting,
            AgentActivityStatus::Waiting,
        ),
        (
            archon_observability::AgentActivityStatus::Backgrounded,
            AgentActivityStatus::Backgrounded,
        ),
        (
            archon_observability::AgentActivityStatus::Cancelled,
            AgentActivityStatus::Cancelled,
        ),
    ];

    for (source, expected) in cases {
        let update = AgentActivityUpdate::from(
            archon_observability::AgentActivityEvent::new(
                "session-1",
                archon_observability::AgentActivityKind::AgentRunning,
                source,
                "state update",
            )
            .with_subagent_id(format!("sub-{expected:?}")),
        );
        assert_eq!(update.status, expected);
    }
}

#[test]
fn app_keeps_artifact_and_run_metadata_from_canonical_activity() {
    let update = AgentActivityUpdate::from(
        archon_observability::AgentActivityEvent::new(
            "session-1",
            archon_observability::AgentActivityKind::ArtifactCreated,
            archon_observability::AgentActivityStatus::Completed,
            "report ready",
        )
        .with_run_id("gt-run-1")
        .with_parent_id("parent-turn-1")
        .with_artifact_id("artifact-report-1"),
    );

    assert_eq!(update.run_id.as_deref(), Some("gt-run-1"));
    assert_eq!(update.parent_id.as_deref(), Some("parent-turn-1"));
    assert_eq!(update.artifact_id.as_deref(), Some("artifact-report-1"));
}

#[test]
fn app_tool_failure_shows_in_output() {
    let mut app = App::new();
    app.on_tool_start("Bash", "tool-456");
    app.on_tool_complete("Bash", "tool-456", false, "command not found");
    // Failed tool calls DO show in output
    assert!(
        app.output
            .all_lines()
            .iter()
            .any(|l| l.contains("Bash") && l.contains("failed"))
    );
}

#[test]
fn thinking_delta_does_not_pollute_output() {
    let mut app = App::new();
    app.show_thinking = true;
    app.on_thinking_delta("I am pondering...");
    // Output buffer should be empty — thinking goes to ThinkingState
    assert!(app.output.all_lines().is_empty());
    assert!(app.thinking.active);
    assert_eq!(app.thinking.accumulated, "I am pondering...");
}

#[test]
fn thinking_tracks_timing_even_when_hidden() {
    let mut app = App::new();
    // show_thinking is false by default
    app.on_thinking_delta("hidden thought");
    assert!(app.thinking.active);
    assert!(app.thinking.start.is_some());
    // Text NOT accumulated when hidden
    assert!(app.thinking.accumulated.is_empty());
}

#[test]
fn thinking_completes_on_text_delta() {
    let mut app = App::new();
    app.show_thinking = true;
    app.on_thinking_delta("deep thought");
    assert!(app.thinking.active);
    app.on_text_delta("answer");
    // Thinking should now be complete; summary is rendered by
    // thinking_lines(), NOT appended to the output buffer.
    assert!(!app.thinking.active);
    let lines = app.output.all_lines();
    assert!(!lines.iter().any(|l| l.contains("Thought for")));
    assert!(lines.iter().any(|l| l.contains("answer")));
}

#[test]
fn thinking_completes_on_turn_complete() {
    let mut app = App::new();
    app.on_thinking_delta("pondering");
    app.on_turn_complete();
    assert!(!app.thinking.active);
    // Summary is rendered separately — not in the output buffer.
    let lines = app.output.all_lines();
    assert!(!lines.iter().any(|l| l.contains("Thought for")));
}

#[test]
fn submit_input_never_sets_is_generating() {
    // No input — slash or normal — should set is_generating in submit_input.
    // The flag is controlled exclusively by GenerationStarted/TurnComplete events.
    let cases = vec![
        "hello world",
        "/model opus",
        "/fast",
        "/gibberish",
        "/",
        "/ help",
        "/usr/bin/foo",
        "/etc/hosts",
    ];
    for input in cases {
        let mut app = App::new();
        for c in input.chars() {
            app.input.insert(c);
        }
        let text = app.submit_input();
        assert_eq!(text, input);
        assert!(
            !app.is_generating,
            "submit_input set is_generating for '{input}'"
        );
    }
}

#[test]
fn generation_started_sets_is_generating() {
    let mut app = App::new();
    assert!(!app.is_generating);
    app.on_generation_started();
    assert!(app.is_generating);
}

#[test]
fn slash_command_complete_resets_is_generating() {
    let mut app = App::new();
    app.on_slash_command_complete();
    assert!(!app.is_generating);
}

#[test]
fn input_accepts_paste_only_without_modal_overlays() {
    let mut app = App::new();
    assert!(app.input_accepts_paste());

    app.btw_overlay = Some("side question".into());
    assert!(!app.input_accepts_paste());

    app.btw_overlay = None;
    app.permission_prompt = Some("Bash".into());
    assert!(!app.input_accepts_paste());
}

#[test]
fn full_agent_turn_lifecycle() {
    // Simulates: user submits -> GenerationStarted -> TextDelta -> TurnComplete
    let mut app = App::new();
    for c in "hello".chars() {
        app.input.insert(c);
    }
    app.submit_input();
    assert!(!app.is_generating); // submit_input does NOT set it

    app.on_generation_started();
    assert!(app.is_generating); // now set by event

    app.on_text_delta("response");
    assert!(app.is_generating); // still generating during response

    app.on_turn_complete();
    assert!(!app.is_generating); // reset after turn completes
}

#[test]
fn slash_command_lifecycle() {
    // Simulates: user submits /model -> SlashCommandComplete
    let mut app = App::new();
    for c in "/model opus".chars() {
        app.input.insert(c);
    }
    app.submit_input();
    assert!(!app.is_generating); // never set for slash commands

    // main.rs sends SlashCommandComplete — this is a no-op since
    // is_generating was never true, but it ensures consistency
    app.on_slash_command_complete();
    assert!(!app.is_generating);
}

#[test]
fn unrecognized_slash_command_fallthrough() {
    // Simulates: user types /gibberish -> not handled -> falls through to agent
    let mut app = App::new();
    for c in "/gibberish".chars() {
        app.input.insert(c);
    }
    app.submit_input();
    assert!(!app.is_generating); // submit_input does NOT set it

    // main.rs sends GenerationStarted before agent.process_message()
    app.on_generation_started();
    assert!(app.is_generating); // correctly set for agent turn

    app.on_turn_complete();
    assert!(!app.is_generating);
}

#[test]
fn open_view_sets_docs_evidence_overlay_source_of_truth() {
    let mut app = App::new();
    app.open_view(ViewId::Docs);

    let view = app.evidence_view.as_ref().expect("view opened");
    assert_eq!(view.view_id(), ViewId::Docs);
    assert!(matches!(view, EvidenceViewState::Docs(_)));
}

#[test]
fn open_view_with_rows_sets_docs_rows_from_source_of_truth() {
    let mut app = App::new();
    app.open_view_with_rows(
        ViewId::Docs,
        vec![super::EvidenceRowPayload {
            id: "doc-1".into(),
            title: "Policy Pack".into(),
            status: "Processed".into(),
            detail: "12 chunks".into(),
        }],
    );

    let view = app.evidence_view.as_ref().expect("view opened");
    let EvidenceViewState::Docs(screen) = view else {
        panic!("expected docs view");
    };
    assert_eq!(screen.len(), 1);
    assert_eq!(screen.selected().unwrap().id, "doc-1");
    assert_eq!(screen.selected().unwrap().summary, "12 chunks");
}

#[test]
fn open_view_sets_gametheory_evidence_overlay_source_of_truth() {
    let mut app = App::new();
    app.open_view(ViewId::GameTheory);

    let view = app.evidence_view.as_ref().expect("view opened");
    assert_eq!(view.view_id(), ViewId::GameTheory);
    assert!(matches!(view, EvidenceViewState::GameTheory(_)));
}

#[test]
fn open_view_sets_learning_evidence_overlay_source_of_truth() {
    let mut app = App::new();
    app.open_view(ViewId::Learning);

    let view = app.evidence_view.as_ref().expect("view opened");
    assert_eq!(view.view_id(), ViewId::Learning);
    assert!(matches!(view, EvidenceViewState::Learning(_)));
}
