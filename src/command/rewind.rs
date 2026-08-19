//! TASK-TUI-620 /rewind slash-command handler.
//!
//! `/rewind` opens a message-selector overlay listing prior conversation
//! messages; user picks an index and the conversation is truncated to
//! that point.
//!
//! # Architecture (overlay command)
//!
//! Unlike the Phase B DIRECT commands (/teleport, /review, /commit, /tag)
//! which emit a single `TextDelta`, `/rewind` emits a dedicated
//! `TuiEvent::ShowMessageSelector(Vec<MessageSummary>)` variant that the
//! event loop handles by setting `app.message_selector = Some(...)`.
//!
//! # Reconciliation with TASK-TUI-620.md spec
//!
//! Spec references `crates/archon-tui/src/slash/rewind.rs` +
//! `SlashCommand` + `SlashOutcome::OpenOverlay(Box::new(MessageSelectorOverlay))`.
//! Actual: bin-crate `src/command/rewind.rs` + `CommandHandler` + a
//! dedicated `TuiEvent::ShowMessageSelector` variant (per Kimi K2's
//! prep plan choosing the existing ad-hoc overlay pattern).
//!
//! Spec proposes `crates/archon-tui/src/overlays/message_selector.rs`.
//! The actual screens/ directory already contains 13 overlays
//! (`session_browser`, `mcp_view`, `model_picker`, etc.) at
//! `crates/archon-tui/src/screens/`. Gate 2 adds
//! `screens/message_selector.rs` there, NOT a new `overlays/` directory.
//!
//! # Message-history access
//!
//! `ctx.session_id` may be `None` (no active session). When None, `/rewind`
//! returns `Err("no active session")`. When Some, the handler loads the
//! session's messages from the shared configured `SessionStore` carried in the
//! command context. Empty list => `Err("no messages to rewind to")`.
//!
//! # Gate 2 scope
//!
//! - `MessageLoader` trait seam (like `TagStore`) so tests inject a
//!   `MockMessageLoader` without instantiating Cozo.
//! - `RewindHandler` with `new()` and `with_loader(...)` (test injection).
//! - 3 tests exercising the three branches (no session_id, no messages,
//!   happy path emits `ShowMessageSelector`).
//!
//! # Follow-up work — landed
//!
//! This module used to list three items as deferred under a
//! `TUI-620-followup` marker. All three are in the tree:
//!
//! - Full ratatui render of `MessageSelector` — `draw_message_selector` in
//!   `crates/archon-tui/src/render/body/pickers.rs`.
//! - Input priority-branch routing — the `app.message_selector.is_some()`
//!   branch in `crates/archon-tui/src/event_loop/input.rs`.
//! - Truncate-on-confirm — the `__truncate_session__` control input in
//!   `src/session_loop/control_input.rs`, handled by
//!   `session_loop::session_history::handle_truncate_session`.

use archon_tui::app::{MessageSummary, TuiEvent};

use crate::command::registry::{CommandContext, CommandHandler};

/// Seam — tests inject a `MockMessageLoader`. Returns a
/// `Result<Vec<MessageSummary>, String>` so the handler can surface loader
/// errors via `anyhow::anyhow!(e)`.
pub(crate) trait MessageLoader: Send + Sync {
    fn load(&self, session_id: &str) -> Result<Vec<MessageSummary>, String>;
}

fn load_message_summaries(
    store: &archon_session::storage::SessionStore,
    session_id: &str,
) -> Result<Vec<MessageSummary>, String> {
    let raw = store
        .load_messages(session_id)
        .map_err(|e| format!("load_messages failed: {e}"))?;

    let mut out: Vec<MessageSummary> = Vec::new();
    for (idx, raw_msg) in raw.iter().enumerate() {
        let value: serde_json::Value = match serde_json::from_str(raw_msg) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed rows
        };

        // Same content-extraction shape as the /resume path in
        // src/session.rs around line 2123 — handle String and
        // Array-of-{text} forms.
        let content = match &value["content"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|item| item["text"].as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if content.is_empty() {
            continue;
        }

        let timestamp = value["timestamp"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        let preview: String = content.chars().take(80).collect();

        out.push(MessageSummary {
            id: format!("msg-{idx:03}"),
            timestamp,
            preview,
        });
    }

    Ok(out)
}

pub(crate) struct RewindHandler {
    loader: Option<std::sync::Arc<dyn MessageLoader>>,
}

impl RewindHandler {
    pub(crate) fn new() -> Self {
        Self { loader: None }
    }

    #[cfg(test)]
    pub(crate) fn with_loader(loader: std::sync::Arc<dyn MessageLoader>) -> Self {
        Self {
            loader: Some(loader),
        }
    }
}

impl CommandHandler for RewindHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let session_id = ctx.session_id.clone().ok_or_else(|| {
            anyhow::anyhow!("no active session — start a session before using /rewind")
        })?;

        let messages = if let Some(store) = ctx.session_store.as_deref() {
            load_message_summaries(store, &session_id).map_err(|e| anyhow::anyhow!(e))?
        } else {
            self.loader
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("session store is unavailable"))?
                .load(&session_id)
                .map_err(|e| anyhow::anyhow!(e))?
        };

        if messages.is_empty() {
            return Err(anyhow::anyhow!("no messages to rewind to"));
        }

        ctx.emit(TuiEvent::ShowMessageSelector(messages));
        Ok(())
    }

    fn description(&self) -> &str {
        "Open message selector to rewind conversation to a prior point"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;
    use chrono::Utc;
    use std::sync::Arc;

    struct MockMessageLoader {
        messages: Vec<MessageSummary>,
    }

    impl MessageLoader for MockMessageLoader {
        fn load(&self, _session_id: &str) -> Result<Vec<MessageSummary>, String> {
            Ok(self.messages.clone())
        }
    }

    fn ctx_with_session() -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
        let (mut ctx, rx) = make_bug_ctx();
        ctx.session_id = Some("test-session-abc".to_string());
        (ctx, rx)
    }

    fn fixture_messages(n: usize) -> Vec<MessageSummary> {
        (0..n)
            .map(|i| MessageSummary {
                id: format!("msg-{}", i),
                timestamp: Utc::now(),
                preview: format!("preview-{}", i),
            })
            .collect()
    }

    #[test]
    fn no_session_id_returns_err() {
        let loader = Arc::new(MockMessageLoader {
            messages: fixture_messages(5),
        });
        let handler = RewindHandler::with_loader(loader);
        let (mut ctx, _rx) = make_bug_ctx(); // session_id None
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("session"),
            "expected 'session' in err; got: {}",
            msg
        );
    }

    #[test]
    fn no_messages_returns_err() {
        let loader = Arc::new(MockMessageLoader { messages: vec![] });
        let handler = RewindHandler::with_loader(loader);
        let (mut ctx, _rx) = ctx_with_session();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("no messages") || msg.contains("empty"),
            "expected 'no messages' or 'empty'; got: {}",
            msg
        );
    }

    #[test]
    fn with_messages_emits_show_message_selector() {
        let loader = Arc::new(MockMessageLoader {
            messages: fixture_messages(5),
        });
        let handler = RewindHandler::with_loader(loader);
        let (mut ctx, mut rx) = ctx_with_session();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::ShowMessageSelector(msgs) => {
                assert_eq!(msgs.len(), 5, "expected 5 messages, got {}", msgs.len());
            }
            other => panic!("expected ShowMessageSelector, got {:?}", other),
        }
    }

    /// Dispatch reaches the registered handler, proven on the two `Err` paths
    /// that need no real (Cozo-backed) `SessionStore`.
    ///
    /// This was `#[ignore]`d as a "live smoke" test, and had been wrong for as
    /// long as it was ignored: it asserted the second path produced "no
    /// messages to rewind to". That stopped being true when `RealMessageLoader`
    /// was replaced by `load_message_summaries` reading `ctx.session_store`
    /// directly. Through `default_registry()` the handler carries no fallback
    /// loader and `make_bug_ctx()` carries no store, so the reachable second
    /// error is "session store is unavailable". Nothing ran it, so nothing said
    /// so. It now asserts what the code does, and is no longer ignored.
    ///
    /// The empty-message branch it *meant* to cover is covered by
    /// `no_messages_returns_err`, which injects an empty `MockMessageLoader`.
    #[test]
    fn rewind_dispatches_via_registry() {
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("rewind")
            .expect("rewind must be registered in default_registry()");

        // Path 1: no session_id.
        let (mut ctx, mut rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err(), "no-session path must Err");
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("session"),
            "no-session Err must mention 'session'; got: {}",
            msg
        );
        let events = drain_tui_events(&mut rx);
        assert!(
            events.is_empty(),
            "no-session path must emit no events; got: {:?}",
            events
        );

        // Path 2: session_id set, but no store and no injected loader — which
        // is exactly the shape a registry-dispatched handler has outside a
        // live session.
        let (mut ctx2, mut rx2) = make_bug_ctx();
        ctx2.session_id = Some("smoke-session".to_string());
        let result2 = handler.execute(&mut ctx2, &[]);
        assert!(result2.is_err(), "no-store path must Err");
        let msg2 = format!("{:#}", result2.unwrap_err()).to_lowercase();
        assert!(
            msg2.contains("session store"),
            "no-store Err must name the missing store; got: {}",
            msg2
        );
        let events2 = drain_tui_events(&mut rx2);
        assert!(
            events2.is_empty(),
            "no-store path must emit no events; got: {:?}",
            events2
        );
    }
}
