//! TASK-TUI-625 /session slash-command handler.
//!
//! `/session` shows the current remote session URL + a Unicode QR code.
//! Requires `--remote` mode (reads URL via a `RemoteUrlProvider` seam).
//! When not in remote mode, returns `Err("not in remote mode")`.
//!
//! # Architecture
//!
//! Unlike overlay commands (/rewind, /skills), /session emits the QR
//! + URL as a single `TuiEvent::TextDelta` (DIRECT pattern, like
//! /review and /commit). No overlay needed — the QR ASCII art fits in
//! the TUI output pane.
//!
//! # QR rendering
//!
//! Uses the `qrcode` crate (0.14.1, no default features) via the
//! `archon_tui::qr::render_url_as_qr` helper. That helper emits
//! Unicode half-block characters (▀ ▄ █) that render as a clean
//! terminal QR. Encapsulating the `qrcode` dependency inside the
//! archon-tui crate keeps the bin-crate dep graph untouched.
//!
//! # Reconciliation with TASK-TUI-625.md spec
//!
//! 1. Spec path: `crates/archon-tui/src/slash/session.rs` +
//!    `SlashCommand`. Actual: bin-crate `src/command/session.rs` +
//!    `CommandHandler` (same reconciliation as TUI-621..627).
//!
//! 2. Spec: "Remove /session alias from /resume." **Confirmed no-op** —
//!    grep of `src/command/resume.rs` shows `/session` is NOT currently
//!    an alias. The spec's TUI-605 collision-test update is similarly
//!    moot — no change needed.
//!
//! 3. Spec: "Add `remote_session_url: Option<String>` to `AppState`."
//!    Adapted to a `RemoteUrlProvider` trait seam (like `GhRunner` /
//!    `TagStore` / `MessageLoader`) — production reads the env var
//!    `ARCHON_REMOTE_URL`, tests inject `MockRemoteUrlProvider`. Avoids
//!    adding a new AppState field during the sweep; the real startup
//!    wiring has landed — `main.rs` sets `ARCHON_REMOTE_URL` from the
//!    `--remote-url <URL>` CLI flag before any tokio task is spawned,
//!    so `EnvRemoteUrlProvider` sees the value supplied at launch.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Seam — tests inject `MockRemoteUrlProvider`. Production reads the
/// `ARCHON_REMOTE_URL` environment variable, which `main.rs` sets from
/// the `--remote-url <URL>` CLI flag at startup (the TUI-625-followup
/// wiring has landed — see `src/main.rs` just after `Cli::parse()`).
pub(crate) trait RemoteUrlProvider: Send + Sync {
    fn url(&self) -> Option<String>;
}

pub(crate) struct EnvRemoteUrlProvider;

impl RemoteUrlProvider for EnvRemoteUrlProvider {
    fn url(&self) -> Option<String> {
        std::env::var("ARCHON_REMOTE_URL")
            .ok()
            .filter(|s| !s.is_empty())
    }
}

/// `/session` handler — emits QR + URL for the active remote session.
pub(crate) struct SessionHandler {
    provider: std::sync::Arc<dyn RemoteUrlProvider>,
}

impl SessionHandler {
    pub(crate) fn new() -> Self {
        Self {
            provider: std::sync::Arc::new(EnvRemoteUrlProvider),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_provider(provider: std::sync::Arc<dyn RemoteUrlProvider>) -> Self {
        Self { provider }
    }
}

impl CommandHandler for SessionHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let url = self.provider.url().ok_or_else(|| {
            anyhow::anyhow!("not in remote mode — start archon with --remote to use /session")
        })?;

        let qr_art = archon_tui::qr::render_url_as_qr(&url)
            .map_err(|e| anyhow::anyhow!("QR render failed: {}", e))?;

        let message = format!(
            "\n/session — remote session\n\n{}\n\nOpen in browser: {}\n(press q to dismiss)\n",
            qr_art, url,
        );
        ctx.emit(TuiEvent::TextDelta(message));
        Ok(())
    }

    fn description(&self) -> &str {
        "Show remote session QR code and URL (requires --remote)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;
    use std::sync::Arc;

    struct MockRemoteUrlProvider {
        url: Option<String>,
    }
    impl RemoteUrlProvider for MockRemoteUrlProvider {
        fn url(&self) -> Option<String> {
            self.url.clone()
        }
    }

    #[test]
    fn no_remote_url_returns_err() {
        let provider = Arc::new(MockRemoteUrlProvider { url: None });
        let handler = SessionHandler::with_provider(provider);
        let (mut ctx, _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err()).to_lowercase();
        assert!(
            msg.contains("remote") || msg.contains("--remote"),
            "expected 'remote' in err; got: {}",
            msg
        );
    }

    #[test]
    fn with_remote_url_emits_textdelta_with_url() {
        let url = "https://archon.example.test/sess/abc123".to_string();
        let provider = Arc::new(MockRemoteUrlProvider {
            url: Some(url.clone()),
        });
        let handler = SessionHandler::with_provider(provider);
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains(&url),
                    "expected URL '{}' in TextDelta; got: {}",
                    url,
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn with_remote_url_emits_textdelta_with_qr_art() {
        let provider = Arc::new(MockRemoteUrlProvider {
            url: Some("https://archon.example.test/sess/qr-test".to_string()),
        });
        let handler = SessionHandler::with_provider(provider);
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                // QR rendered as Unicode half-blocks — at least one
                // of ▀ ▄ █ must appear.
                assert!(
                    s.contains('\u{2580}') || s.contains('\u{2584}') || s.contains('\u{2588}'),
                    "expected Unicode half-block QR art; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn env_provider_reads_set_var() {
        // Save prior value, set a test URL, read via EnvRemoteUrlProvider.
        let prior = std::env::var("ARCHON_REMOTE_URL").ok();
        unsafe {
            std::env::set_var("ARCHON_REMOTE_URL", "https://archon.example/sess/env-test");
        }
        let provider = EnvRemoteUrlProvider;
        let got = provider.url();
        // Restore prior — best-effort.
        match prior {
            Some(v) => unsafe {
                std::env::set_var("ARCHON_REMOTE_URL", v);
            },
            None => unsafe {
                std::env::remove_var("ARCHON_REMOTE_URL");
            },
        }
        assert_eq!(got.as_deref(), Some("https://archon.example/sess/env-test"));
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn session_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("session") must return Some(handler).
        // The registered handler uses EnvRemoteUrlProvider reading
        // ARCHON_REMOTE_URL. Accept BOTH outcomes:
        //   (a) env var unset: Err containing "remote" + no events emitted.
        //   (b) env var set: Ok + TextDelta containing "/session" header.
        // Either path proves dispatch wiring + env-seam work end-to-end.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("session")
            .expect("session must be registered in default_registry()");

        // Ensure the env var is in a known state (unset) at start of test.
        // SAFETY: remove_var is unsafe in Rust 1.77+ due to multi-threading
        // concerns; this test is single-threaded within its test-threads=2
        // allocation and the var is not read by other tests.
        let prior = std::env::var("ARCHON_REMOTE_URL").ok();
        unsafe {
            std::env::remove_var("ARCHON_REMOTE_URL");
        }

        let (mut ctx, mut rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[]);

        // Restore prior env value (best-effort).
        if let Some(v) = prior {
            unsafe {
                std::env::set_var("ARCHON_REMOTE_URL", v);
            }
        }

        match result {
            Ok(()) => {
                // Only reachable if env leaked a value (prior line didn't unset).
                // Still acceptable — we proved dispatch works. Just assert the
                // TextDelta carries the /session header.
                let events = drain_tui_events(&mut rx);
                assert_eq!(events.len(), 1);
                match &events[0] {
                    TuiEvent::TextDelta(s) => {
                        assert!(
                            s.to_lowercase().contains("session"),
                            "Ok path TextDelta must carry /session header; got: {}",
                            s
                        );
                    }
                    other => panic!("expected TextDelta on Ok path, got: {:?}", other),
                }
            }
            Err(e) => {
                let msg = format!("{:#}", e).to_lowercase();
                assert!(
                    msg.contains("remote"),
                    "Err path must mention 'remote'; got: {}",
                    msg
                );
                let events = drain_tui_events(&mut rx);
                assert!(
                    events.is_empty(),
                    "Err path must not emit any events; got: {:?}",
                    events
                );
            }
        }
    }
}
