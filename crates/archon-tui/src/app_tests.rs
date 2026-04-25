//! Unit tests for `App` state transitions (TUI-310 extraction).
//!
//! Moved from `app.rs` as part of the event-loop extraction so `app.rs`
//! can remain a thin orchestrator module (<500 lines). These tests
//! exercise `App` methods — no event-loop coverage here (that lives in
//! `tests/event_loop_smoke.rs` and `tests/app_run_e2e.rs`).

use super::App;

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
