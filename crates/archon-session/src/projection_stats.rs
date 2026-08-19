//! Session statistics as a projection (#193 Phase B, #192).
//!
//! The migrated consumer that proves the machinery. It also settles what to do
//! with `archon-tui`'s `session_stats.rs`, which was never a screen: it had no
//! `render`, it recomputed everything by rescanning the whole log on every
//! call, and it carried a `StatsSource`/`NullStats` shim its own comment called
//! temporary. Counting messages is a fold, and this is what a fold looks like.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::projection::{SessionEvent, SessionProjection};

/// What a session's log adds up to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStatsState {
    pub message_count: u64,
    pub user_messages: u64,
    pub assistant_messages: u64,
    /// Distinct tool names seen, in name order.
    ///
    /// A set rather than a count because "which tools did this session use" is
    /// the question people actually ask, and a count cannot be refined into it
    /// later without refolding.
    pub tools_used: BTreeSet<String>,
}

/// Folds a session log into [`SessionStatsState`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SessionStatsProjection;

impl SessionProjection for SessionStatsProjection {
    type State = SessionStatsState;

    fn key(&self) -> &'static str {
        "session_stats.v1"
    }

    fn init(&self) -> Self::State {
        SessionStatsState::default()
    }

    fn apply(&self, state: Arc<Self::State>, event: &SessionEvent) -> Arc<Self::State> {
        // Anything that is not a JSON object with a role tells us nothing.
        // Handing back the same Arc is how a unit says so, and it costs the
        // driver one pointer comparison.
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&event.payload) else {
            return state;
        };
        let Some(role) = value.get("role").and_then(serde_json::Value::as_str) else {
            return state;
        };

        let tools = tool_names(&value);
        let mut next = (*state).clone();
        next.message_count += 1;
        match role {
            "user" => next.user_messages += 1,
            "assistant" => next.assistant_messages += 1,
            _ => {}
        }
        next.tools_used.extend(tools);
        Arc::new(next)
    }

    fn view(&self, state: &Self::State) -> serde_json::Value {
        serde_json::json!({
            "messages": state.message_count,
            "user": state.user_messages,
            "assistant": state.assistant_messages,
            "tools": state.tools_used.iter().collect::<Vec<_>>(),
        })
    }
}

/// Tool names named by one message.
///
/// Reads the Anthropic content shape: a list of blocks, each with a `type`, and
/// a `tool_use` block carrying the tool's `name`. A string content block names
/// no tool, which is the common case and returns nothing rather than guessing.
fn tool_names(message: &serde_json::Value) -> Vec<String> {
    message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| {
                    block.get("type").and_then(serde_json::Value::as_str) == Some("tool_use")
                })
                .filter_map(|block| {
                    block
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(seq: u64, payload: &str) -> SessionEvent {
        SessionEvent {
            seq,
            payload: payload.to_string(),
        }
    }

    #[test]
    fn roles_are_counted_separately_and_together() {
        let unit = SessionStatsProjection;
        let mut state = Arc::new(unit.init());
        state = unit.apply(state, &event(0, r#"{"role":"user","content":"hi"}"#));
        state = unit.apply(
            state,
            &event(1, r#"{"role":"assistant","content":"hello"}"#),
        );
        state = unit.apply(state, &event(2, r#"{"role":"user","content":"again"}"#));

        assert_eq!(state.message_count, 3);
        assert_eq!(state.user_messages, 2);
        assert_eq!(state.assistant_messages, 1);
    }

    #[test]
    fn tool_uses_are_collected_by_name() {
        let unit = SessionStatsProjection;
        let state = unit.apply(
            Arc::new(unit.init()),
            &event(
                0,
                r#"{"role":"assistant","content":[
                    {"type":"text","text":"working"},
                    {"type":"tool_use","name":"Bash","input":{}},
                    {"type":"tool_use","name":"Read","input":{}}
                ]}"#,
            ),
        );

        assert_eq!(
            state.tools_used.iter().cloned().collect::<Vec<_>>(),
            vec!["Bash".to_string(), "Read".to_string()]
        );
    }

    /// The rule that makes folding every event through every unit affordable:
    /// a unit that is not interested hands back what it was given, and the
    /// driver can see that without comparing the state itself.
    #[test]
    fn an_uninteresting_event_returns_the_same_state() {
        let unit = SessionStatsProjection;
        let state = Arc::new(unit.init());

        let after_garbage = unit.apply(Arc::clone(&state), &event(0, "not json at all"));
        assert!(Arc::ptr_eq(&state, &after_garbage), "garbage changed state");

        let after_roleless = unit.apply(Arc::clone(&state), &event(1, r#"{"content":"no role"}"#));
        assert!(
            Arc::ptr_eq(&state, &after_roleless),
            "a message with no role changed state"
        );
    }

    #[test]
    fn the_view_names_what_a_client_needs() {
        let unit = SessionStatsProjection;
        let state = unit.apply(
            Arc::new(unit.init()),
            &event(0, r#"{"role":"assistant","content":"x"}"#),
        );

        let view = unit.view(&state);
        assert_eq!(view["messages"], 1);
        assert_eq!(view["assistant"], 1);
        assert_eq!(view["user"], 0);
    }

    /// The persisted cache requires it, so it is worth pinning.
    #[test]
    fn the_state_round_trips_through_json() {
        let unit = SessionStatsProjection;
        let state = unit.apply(
            Arc::new(unit.init()),
            &event(
                0,
                r#"{"role":"assistant","content":[{"type":"tool_use","name":"Grep","input":{}}]}"#,
            ),
        );

        let encoded = serde_json::to_string(&*state).expect("serialise");
        let decoded: SessionStatsState = serde_json::from_str(&encoded).expect("deserialise");

        assert_eq!(decoded, *state);
    }
}
