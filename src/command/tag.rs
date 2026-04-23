//! TASK-TUI-623 /tag slash-command handler.
//!
//! `/tag <name>` toggles a searchable tag on the current session:
//!
//!   - If the tag is not yet on the session: ADD it. Emit
//!     "Tagged session with #<name>".
//!   - If the tag is already on the session: REMOVE it. Emit
//!     "Removed tag #<name>".
//!
//! Errs on empty tag, or if the current session has no ID (no active
//! session).
//!
//! # Reconciliation with TASK-TUI-623.md spec
//!
//! 1. **Storage model adaptation.** The spec proposes
//!    `tag: Option<String>` — a SINGLE tag per session. The actual
//!    `archon-session` crate already implements a MULTI-TAG schema in
//!    the `session_tags` Cozo relation, exposed via:
//!      - `SessionStore::put_tag(session_id, tag) -> Result<(), _>`
//!      - `SessionStore::delete_tag(session_id, tag)`
//!      - `SessionStore::list_tags(session_id) -> Result<Vec<String>, _>`
//!    plus the `metadata::add_tag / remove_tag / get_tags` free-function
//!    wrappers.
//!
//!    The handler enforces SPEC SEMANTICS at the command level while
//!    using the multi-tag storage underneath:
//!      - Empty tag set + add X: `put_tag(X)`.
//!      - Tag set `{X}` + toggle X: `delete_tag(X)` (toggle off).
//!      - Tag set `{X}` + add Y (Y != X): `delete_tag(X)` THEN `put_tag(Y)`
//!        — "replace" semantics per spec's `replace_different_tag` test.
//!      - Tag set with multiple tags + add Y not in set: policy is
//!        "clear all existing then add new" — this preserves spec's
//!        single-visible-tag semantics while not corrupting the underlying
//!        multi-tag schema (other flows can still write tags directly
//!        via `metadata::add_tag`).
//!
//! 2. **Trait surface.** Same as TUI-621..624: spec says
//!    `crates/archon-tui/src/slash/tag.rs + SlashCommand +
//!    SlashOutcome::Message`; actual is bin-crate `src/command/tag.rs` +
//!    `CommandHandler` (re-exported as `SlashCommand` at
//!    `src/command/mod.rs:86`) + `ctx.emit(TuiEvent::TextDelta)`.
//!
//! 3. **Testability seam.** Opening a real `SessionStore` in tests
//!    requires a temp Cozo directory. Gate 2 introduces a `TagStore`
//!    trait with `RealTagStore` (wraps `SessionStore`) and `MockTagStore`
//!    (`#[cfg(test)]`) so unit tests are deterministic and do not
//!    instantiate Cozo. Mirrors the `GhRunner` / `GitRunner` pattern
//!    from TUI-622 / TUI-624.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Session tag storage seam — tests inject `MockTagStore`, production
/// uses `RealTagStore` which wraps `archon_session::storage::SessionStore`.
pub(crate) trait TagStore: Send + Sync {
    fn list_tags(&self, session_id: &str) -> Result<Vec<String>, String>;
    fn put_tag(&self, session_id: &str, tag: &str) -> Result<(), String>;
    fn delete_tag(&self, session_id: &str, tag: &str) -> Result<(), String>;
}

/// Production `TagStore` — opens `SessionStore::open_default()` on each
/// call. Suboptimal for perf but preserves the zero-shared-state handler
/// pattern used by other DIRECT-with-storage handlers (fork, resume).
pub(crate) struct RealTagStore;

impl TagStore for RealTagStore {
    fn list_tags(&self, session_id: &str) -> Result<Vec<String>, String> {
        let store = archon_session::storage::SessionStore::open_default()
            .map_err(|e| format!("failed to open session store: {}", e))?;
        store
            .list_tags(session_id)
            .map_err(|e| format!("list_tags failed: {}", e))
    }
    fn put_tag(&self, session_id: &str, tag: &str) -> Result<(), String> {
        let store = archon_session::storage::SessionStore::open_default()
            .map_err(|e| format!("failed to open session store: {}", e))?;
        store
            .put_tag(session_id, tag)
            .map_err(|e| format!("put_tag failed: {}", e))
    }
    fn delete_tag(&self, session_id: &str, tag: &str) -> Result<(), String> {
        let store = archon_session::storage::SessionStore::open_default()
            .map_err(|e| format!("failed to open session store: {}", e))?;
        store
            .delete_tag(session_id, tag)
            .map_err(|e| format!("delete_tag failed: {}", e))
    }
}

/// Strip ASCII control chars (< 0x20) from `s`, then trim whitespace.
/// Intentionally simple: Cozo stores tags as String; the less sanitization
/// surface the better (we do NOT remove Unicode control chars beyond ASCII
/// since those are legitimately present in some tag-naming conventions).
fn sanitize_tag(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_ascii_control())
        .collect::<String>()
        .trim()
        .to_string()
}

/// `/tag` handler — toggles a tag on the current session.
pub(crate) struct TagHandler {
    store: std::sync::Arc<dyn TagStore>,
}

impl TagHandler {
    pub(crate) fn new() -> Self {
        Self {
            store: std::sync::Arc::new(RealTagStore),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_store(store: std::sync::Arc<dyn TagStore>) -> Self {
        Self { store }
    }
}

impl CommandHandler for TagHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        let raw = args.join(" ");
        let tag = sanitize_tag(&raw);
        if tag.is_empty() {
            return Err(anyhow::anyhow!("tag name cannot be empty"));
        }

        let session_id = ctx
            .session_id
            .clone()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no active session — start a session before using /tag"
                )
            })?;

        let existing = self
            .store
            .list_tags(&session_id)
            .map_err(|e| anyhow::anyhow!(e))?;

        let message = if existing.iter().any(|t| t == &tag) {
            // Toggle off: delete the matching tag.
            self.store
                .delete_tag(&session_id, &tag)
                .map_err(|e| anyhow::anyhow!(e))?;
            format!("Removed tag #{}", tag)
        } else {
            // Replace semantics: clear any existing tags first, then add.
            // Preserves spec's "single visible tag" view while using the
            // multi-tag storage schema.
            for old in &existing {
                self.store
                    .delete_tag(&session_id, old)
                    .map_err(|e| anyhow::anyhow!(e))?;
            }
            self.store
                .put_tag(&session_id, &tag)
                .map_err(|e| anyhow::anyhow!(e))?;
            format!("Tagged session with #{}", tag)
        };

        ctx.emit(TuiEvent::TextDelta(format!("\n{}\n", message)));
        Ok(())
    }

    fn description(&self) -> &str {
        "Toggle a searchable tag on the current session"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;
    use std::sync::{Arc, Mutex};

    /// MockTagStore records every call for assertion + returns canned list_tags.
    #[derive(Default)]
    struct MockTagStore {
        list_tags_return: Vec<String>,
        calls: Mutex<Vec<String>>, // e.g. "list:sid", "put:sid:foo", "delete:sid:foo"
    }

    impl MockTagStore {
        fn new(list_return: Vec<String>) -> Arc<Self> {
            Arc::new(Self {
                list_tags_return: list_return,
                calls: Mutex::new(Vec::new()),
            })
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl TagStore for MockTagStore {
        fn list_tags(&self, session_id: &str) -> Result<Vec<String>, String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("list:{}", session_id));
            Ok(self.list_tags_return.clone())
        }
        fn put_tag(&self, session_id: &str, tag: &str) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("put:{}:{}", session_id, tag));
            Ok(())
        }
        fn delete_tag(&self, session_id: &str, tag: &str) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("delete:{}:{}", session_id, tag));
            Ok(())
        }
    }

    /// Build a ctx with session_id populated. `make_bug_ctx` returns a
    /// ctx with session_id=None; we need to override.
    fn ctx_with_session() -> (CommandContext, tokio::sync::mpsc::Receiver<TuiEvent>) {
        let (mut ctx, rx) = make_bug_ctx();
        ctx.session_id = Some("test-session-123".to_string());
        (ctx, rx)
    }

    #[test]
    fn empty_tag_returns_err() {
        let store = MockTagStore::new(vec![]);
        let handler = TagHandler::with_store(store.clone());
        let (mut ctx, _rx) = ctx_with_session();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("empty") || msg.contains("tag name"),
            "err must indicate empty tag; got: {}",
            msg
        );
        assert!(
            store.calls().is_empty(),
            "no store call on empty-tag Err"
        );
    }

    #[test]
    fn no_session_id_returns_err() {
        let store = MockTagStore::new(vec![]);
        let handler = TagHandler::with_store(store.clone());
        let (mut ctx, _rx) = make_bug_ctx(); // session_id stays None
        let result = handler.execute(&mut ctx, &[String::from("bugfix")]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("session"),
            "err must mention 'session'; got: {}",
            msg
        );
        assert!(
            store.calls().is_empty(),
            "no store call when session_id is None"
        );
    }

    #[test]
    fn add_tag_to_untagged_session() {
        let store = MockTagStore::new(vec![]); // no existing tags
        let handler = TagHandler::with_store(store.clone());
        let (mut ctx, mut rx) = ctx_with_session();
        handler.execute(&mut ctx, &[String::from("bugfix")]).unwrap();
        let calls = store.calls();
        assert!(calls.iter().any(|c| c == "list:test-session-123"));
        assert!(
            calls.iter().any(|c| c == "put:test-session-123:bugfix"),
            "expected put_tag call; got: {:?}",
            calls
        );
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains("Tagged") && s.contains("bugfix"),
                    "expected 'Tagged' and 'bugfix' in TextDelta; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn toggle_removes_existing_tag() {
        let store = MockTagStore::new(vec!["bugfix".to_string()]);
        let handler = TagHandler::with_store(store.clone());
        let (mut ctx, mut rx) = ctx_with_session();
        handler.execute(&mut ctx, &[String::from("bugfix")]).unwrap();
        let calls = store.calls();
        assert!(
            calls.iter().any(|c| c == "delete:test-session-123:bugfix"),
            "expected delete_tag; got: {:?}",
            calls
        );
        // put_tag must NOT be called on toggle-off.
        assert!(
            !calls.iter().any(|c| c.starts_with("put:")),
            "put_tag must not be called on toggle-off; got: {:?}",
            calls
        );
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains("Removed") && s.contains("bugfix"),
                    "expected 'Removed' and 'bugfix'; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn replace_different_tag() {
        // Spec: previous tag "bugfix" must be replaced by new "feature-auth".
        // Semantics (multi-tag storage): delete "bugfix", then put "feature-auth".
        let store = MockTagStore::new(vec!["bugfix".to_string()]);
        let handler = TagHandler::with_store(store.clone());
        let (mut ctx, mut rx) = ctx_with_session();
        handler
            .execute(&mut ctx, &[String::from("feature-auth")])
            .unwrap();
        let calls = store.calls();
        assert!(
            calls.iter().any(|c| c == "delete:test-session-123:bugfix"),
            "expected delete_tag for old 'bugfix'; got: {:?}",
            calls
        );
        assert!(
            calls
                .iter()
                .any(|c| c == "put:test-session-123:feature-auth"),
            "expected put_tag for new 'feature-auth'; got: {:?}",
            calls
        );
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains("Tagged") && s.contains("feature-auth"),
                    "expected 'Tagged' and 'feature-auth'; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn unicode_sanitization_strips_control_chars() {
        // NUL + ESC control chars should be stripped; result is "bugfix".
        let store = MockTagStore::new(vec![]);
        let handler = TagHandler::with_store(store.clone());
        let (mut ctx, mut _rx) = ctx_with_session();
        handler
            .execute(&mut ctx, &[String::from("bug\x00fix\x1b")])
            .unwrap();
        let calls = store.calls();
        assert!(
            calls.iter().any(|c| c == "put:test-session-123:bugfix"),
            "expected sanitized tag 'bugfix'; got: {:?}",
            calls
        );
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn tag_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("tag") must return Some(handler) because
        // default_registry() registers TagHandler::new() (with the real
        // RealTagStore). To avoid hitting Cozo, exercise the two pre-storage
        // Err paths:
        //   - args=[] -> empty-tag Err (short-circuits before session check)
        //   - args=["foo"] with default bug ctx (session_id None) -> no-session Err
        // Both prove the dispatcher ran the handler; neither touches Cozo.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("tag")
            .expect("tag must be registered in default_registry()");

        // Path 1: empty args -> "tag name cannot be empty"
        let (mut ctx, mut rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err(), "empty args should Err");
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("empty") || msg.contains("tag name"),
            "empty-args Err must mention 'empty' or 'tag name'; got: {}",
            msg
        );
        let events = drain_tui_events(&mut rx);
        assert!(
            events.is_empty(),
            "empty-args path must emit no events; got: {:?}",
            events
        );

        // Path 2: non-empty args + no session_id -> "no active session"
        let (mut ctx2, mut rx2) = make_bug_ctx(); // session_id None
        let result2 = handler.execute(&mut ctx2, &[String::from("smoketag")]);
        assert!(result2.is_err(), "no-session-id should Err");
        let msg2 = format!("{:#}", result2.unwrap_err()).to_lowercase();
        assert!(
            msg2.contains("session"),
            "no-session-id Err must mention 'session'; got: {}",
            msg2
        );
        let events2 = drain_tui_events(&mut rx2);
        assert!(
            events2.is_empty(),
            "no-session-id path must emit no events; got: {:?}",
            events2
        );
    }
}
