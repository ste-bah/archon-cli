//! TASK-AGS-103: Consumer-side back-pressure EventCoalescer (REQ-FOR-D3 [2/4]).
//!
//! Unit + grep-level regression tests for the render-loop back-pressure
//! policy. Written BEFORE the implementation (Gate 1). The module is
//! exposed from the `archon-cli-workspace` bin crate as `event_coalescer`
//! via `src/lib.rs`.

use std::fs;
use std::path::PathBuf;

use archon_cli_workspace::event_coalescer::{
    EventCoalescer, HARD_CAP, Priority, RENDER_EVENT_BUDGET, SOFT_CAP, priority,
};
use archon_core::agent::AgentEvent;
use archon_tools::tool::ToolResult;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn text_delta(s: &str) -> AgentEvent {
    AgentEvent::TextDelta(s.to_string())
}

fn state_event_error(msg: &str) -> AgentEvent {
    AgentEvent::Error(msg.to_string())
}

// ---------- priority() classification ----------

#[test]
fn priority_classifies_deltas_as_progress() {
    assert_eq!(priority(&text_delta("x")), Priority::Progress);
    assert_eq!(
        priority(&AgentEvent::ThinkingDelta("y".into())),
        Priority::Progress
    );
}

#[test]
fn priority_classifies_state_transitions_as_state() {
    assert_eq!(priority(&AgentEvent::UserPromptReady), Priority::State);
    assert_eq!(
        priority(&AgentEvent::ApiCallStarted { model: "m".into() }),
        Priority::State
    );
    assert_eq!(
        priority(&AgentEvent::ToolCallStarted {
            name: "t".into(),
            id: "1".into()
        }),
        Priority::State
    );
    assert_eq!(
        priority(&AgentEvent::ToolCallComplete {
            name: "t".into(),
            id: "1".into(),
            result: ToolResult {
                content: "ok".into(),
                is_error: false,
            },
        }),
        Priority::State
    );
    assert_eq!(
        priority(&AgentEvent::PermissionRequired {
            tool: "t".into(),
            description: "d".into()
        }),
        Priority::State
    );
    assert_eq!(
        priority(&AgentEvent::PermissionGranted { tool: "t".into() }),
        Priority::State
    );
    assert_eq!(
        priority(&AgentEvent::PermissionDenied { tool: "t".into() }),
        Priority::State
    );
    assert_eq!(
        priority(&AgentEvent::TurnComplete {
            input_tokens: 1,
            output_tokens: 2,
        }),
        Priority::State
    );
    assert_eq!(priority(&state_event_error("e")), Priority::State);
    assert_eq!(priority(&AgentEvent::CompactionTriggered), Priority::State);
    assert_eq!(priority(&AgentEvent::SessionComplete), Priority::State);
    assert_eq!(
        priority(&AgentEvent::AskUser {
            question: "q".into()
        }),
        Priority::State
    );
    assert_eq!(
        priority(&AgentEvent::MessageSent {
            target_agent_id: "a".into(),
            message: "m".into(),
        }),
        Priority::State
    );
}

// ---------- EventCoalescer behaviour ----------

#[test]
fn coalescer_preserves_all_state_events_under_overflow() {
    let mut c = EventCoalescer::with_defaults();
    for i in 0..10_010 {
        c.push(text_delta(&format!("p{i}")));
    }
    c.push(state_event_error("critical-1"));
    c.push(state_event_error("critical-2"));
    c.push(state_event_error("critical-3"));
    c.push(state_event_error("critical-4"));
    c.push(state_event_error("critical-5"));

    // Drain everything.
    let mut state_kept = 0usize;
    while let Some(ev) = c.pop() {
        if priority(&ev) == Priority::State {
            state_kept += 1;
        }
    }
    assert_eq!(
        state_kept, 5,
        "all 5 State events must survive under 10k Progress overflow"
    );
}

#[test]
fn coalescer_drops_oldest_progress_first() {
    let mut c = EventCoalescer::new(5, 10);
    c.push(text_delta("p0"));
    c.push(text_delta("p1"));
    c.push(state_event_error("S"));
    c.push(text_delta("p2"));
    // Push enough to overflow hard cap (10): buffer is already 4, need >10.
    for i in 3..=11 {
        c.push(text_delta(&format!("p{i}")));
    }
    // After overflow, the oldest Progress ("p0") should have been dropped.
    let drained: Vec<_> = std::iter::from_fn(|| c.pop()).collect();
    let p0_present = drained
        .iter()
        .any(|e| matches!(e, AgentEvent::TextDelta(s) if s == "p0"));
    assert!(!p0_present, "oldest Progress (p0) must have been dropped");
    // State event must still be present.
    let s_present = drained
        .iter()
        .any(|e| matches!(e, AgentEvent::Error(s) if s == "S"));
    assert!(s_present, "State event must be preserved");
}

#[test]
fn coalescer_pop_is_fifo() {
    let mut c = EventCoalescer::with_defaults();
    c.push(text_delta("a"));
    c.push(text_delta("b"));
    c.push(text_delta("c"));
    assert!(matches!(c.pop(), Some(AgentEvent::TextDelta(s)) if s == "a"));
    assert!(matches!(c.pop(), Some(AgentEvent::TextDelta(s)) if s == "b"));
    assert!(matches!(c.pop(), Some(AgentEvent::TextDelta(s)) if s == "c"));
    assert!(c.pop().is_none());
}

#[test]
fn coalescer_len_tracks_buffer() {
    let mut c = EventCoalescer::with_defaults();
    assert!(c.is_empty());
    c.push(text_delta("x"));
    c.push(text_delta("y"));
    assert_eq!(c.len(), 2);
    c.pop();
    assert_eq!(c.len(), 1);
}

#[test]
fn constants_match_spec() {
    assert_eq!(SOFT_CAP, 1_000, "spec: soft cap = 1_000");
    assert_eq!(HARD_CAP, 10_000, "spec: hard cap = 10_000");
    assert_eq!(RENDER_EVENT_BUDGET, 10_000, "spec: budget = 10_000");
}

// ---------- wiring grep regression ----------

#[test]
fn main_rs_wires_coalescer_into_render_loop() {
    let src = fs::read_to_string(repo_root().join("src/main.rs")).expect("read main.rs");
    assert!(
        src.contains("event_coalescer"),
        "src/main.rs must import event_coalescer module"
    );
    assert!(
        src.contains("RENDER_EVENT_BUDGET"),
        "src/main.rs must reference RENDER_EVENT_BUDGET constant"
    );
    assert!(
        src.contains("EventCoalescer"),
        "src/main.rs must instantiate an EventCoalescer"
    );
}
