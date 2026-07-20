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
fn priority_classifies_deltas_by_loss_policy() {
    assert_eq!(priority(&text_delta("x")), Priority::Text);
    assert_eq!(
        priority(&AgentEvent::ThinkingDelta("y".into())),
        Priority::Text
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
            transcript_summary: None,
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
        priority(&AgentEvent::PermissionDenied {
            tool: "t".into(),
            reason: None,
        }),
        Priority::State
    );
    assert_eq!(
        priority(&AgentEvent::TurnComplete {
            input_tokens: 1,
            output_tokens: 2,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
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
        c.push(AgentEvent::ThinkingDelta(format!("p{i}")));
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
fn coalescer_preserves_thinking_and_state_under_overflow() {
    let mut c = EventCoalescer::new(5, 10);
    c.push(AgentEvent::ThinkingDelta("p0".into()));
    c.push(AgentEvent::ThinkingDelta("p1".into()));
    c.push(state_event_error("S"));
    c.push(AgentEvent::ThinkingDelta("p2".into()));
    for i in 3..=11 {
        c.push(AgentEvent::ThinkingDelta(format!("p{i}")));
    }

    assert_eq!(c.len(), 3);
    assert!(matches!(c.pop(), Some(AgentEvent::ThinkingDelta(text)) if text == "p0p1"));
    assert!(matches!(c.pop(), Some(AgentEvent::Error(text)) if text == "S"));
    assert!(
        matches!(c.pop(), Some(AgentEvent::ThinkingDelta(text)) if text == "p2p3p4p5p6p7p8p9p10p11")
    );
}

#[test]
fn coalescer_pop_preserves_text_bytes_in_fifo_order() {
    let mut c = EventCoalescer::with_defaults();
    c.push(text_delta("a"));
    c.push(text_delta("b"));
    c.push(text_delta("c"));
    assert!(matches!(c.pop(), Some(AgentEvent::TextDelta(s)) if s == "abc"));
    assert!(c.pop().is_none());
}

#[test]
fn coalescer_len_tracks_coalesced_buffer_entries() {
    let mut c = EventCoalescer::with_defaults();
    assert!(c.is_empty());
    c.push(text_delta("x"));
    c.push(text_delta("y"));
    assert_eq!(c.len(), 1);
    c.pop();
    assert_eq!(c.len(), 0);
}

#[test]
fn constants_match_spec() {
    assert_eq!(SOFT_CAP, 1_000, "spec: soft cap = 1_000");
    assert_eq!(HARD_CAP, 10_000, "spec: hard cap = 10_000");
    assert_eq!(RENDER_EVENT_BUDGET, 10_000, "spec: budget = 10_000");
}

#[test]
fn coalescer_coalesces_adjacent_text_without_losing_bytes() {
    let mut c = EventCoalescer::new(2, 3);
    c.push(text_delta("hello "));
    c.push(text_delta("世界"));

    assert_eq!(c.len(), 1);
    assert!(matches!(c.pop(), Some(AgentEvent::TextDelta(text)) if text == "hello 世界"));
}

#[test]
fn coalescer_coalesces_adjacent_thinking_without_losing_bytes() {
    let mut c = EventCoalescer::new(1, 1);
    let chunks = ["reason ", "世界", "\n", "final"];
    for chunk in chunks {
        c.push(AgentEvent::ThinkingDelta(chunk.into()));
    }

    assert_eq!(c.len(), 1);
    assert!(matches!(c.pop(), Some(AgentEvent::ThinkingDelta(text)) if text == chunks.concat()));
}

#[test]
fn coalescer_preserves_text_across_state_boundaries_under_overflow() {
    let mut c = EventCoalescer::new(2, 3);
    c.push(text_delta("before"));
    c.push(state_event_error("state"));
    c.push(AgentEvent::ThinkingDelta("ephemeral".into()));
    c.push(text_delta("after"));

    let drained: Vec<_> = std::iter::from_fn(|| c.pop()).collect();
    assert_eq!(drained.len(), 4);
    assert!(matches!(&drained[0], AgentEvent::TextDelta(text) if text == "before"));
    assert!(matches!(&drained[1], AgentEvent::Error(text) if text == "state"));
    assert!(matches!(&drained[2], AgentEvent::ThinkingDelta(text) if text == "ephemeral"));
    assert!(matches!(&drained[3], AgentEvent::TextDelta(text) if text == "after"));
}

#[test]
fn coalescer_allows_lossless_events_above_hard_cap() {
    let mut c = EventCoalescer::new(1, 1);
    c.push(text_delta("answer"));
    c.push(state_event_error("done"));

    assert_eq!(c.len(), 2);
    assert!(matches!(c.pop(), Some(AgentEvent::TextDelta(text)) if text == "answer"));
    assert!(matches!(c.pop(), Some(AgentEvent::Error(text)) if text == "done"));
}

#[test]
fn coalescer_soft_cap_allows_lossless_content_overflow() {
    let mut c = EventCoalescer::new(3, 10);
    c.push(text_delta("answer"));
    c.push(state_event_error("state"));
    for index in 0..5 {
        c.push(AgentEvent::ThinkingDelta(format!("thinking-{index}")));
    }

    assert_eq!(c.len(), 3);
    assert!(matches!(c.pop(), Some(AgentEvent::TextDelta(text)) if text == "answer"));
    assert!(matches!(c.pop(), Some(AgentEvent::Error(text)) if text == "state"));
    assert!(
        matches!(c.pop(), Some(AgentEvent::ThinkingDelta(text)) if text == "thinking-0thinking-1thinking-2thinking-3thinking-4")
    );
}

#[test]
fn coalescer_reconstructs_exact_text_burst_across_boundaries() {
    let mut c = EventCoalescer::new(2, 3);
    c.push(text_delta("α"));
    c.push(text_delta("-"));
    c.push(state_event_error("boundary"));
    c.push(text_delta("世界"));
    c.push(text_delta("\nfinal"));

    let drained: Vec<_> = std::iter::from_fn(|| c.pop()).collect();
    assert_eq!(drained.len(), 3);
    assert!(matches!(&drained[0], AgentEvent::TextDelta(text) if text == "α-"));
    assert!(matches!(&drained[1], AgentEvent::Error(text) if text == "boundary"));
    assert!(matches!(&drained[2], AgentEvent::TextDelta(text) if text == "世界\nfinal"));
}

// ---------- wiring grep regression ----------

#[test]
fn event_forwarder_wires_coalescer_into_live_agent_path() {
    let src = fs::read_to_string(repo_root().join("src/session/event_forwarder.rs"))
        .expect("read event forwarder");
    assert!(
        src.contains("EventCoalescer::with_defaults()"),
        "live agent forwarder must instantiate EventCoalescer"
    );
    assert!(
        src.contains("RENDER_EVENT_BUDGET"),
        "live agent forwarder must enforce the render drain budget"
    );
    assert!(
        src.contains("AgentEvent::ThinkingDelta(text) => TuiEvent::ThinkingDelta(text)"),
        "live agent forwarder must preserve thinking deltas"
    );
}
