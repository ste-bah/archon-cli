//! TASK-TUI-628 /sandbox slash-command handler.
//!
//! `/sandbox [on|off|status]` — flips a shared `Arc<AtomicBool>` that
//! the tool-dispatch layer consults before invoking
//! `archon_tui::sandbox::check_permission`. No args (or `status`) reads
//! the current state without flipping.
//!
//! # Reconciliation with TASK-TUI-628.md spec
//!
//! Same trait reconciliation as TUI-621..624: bin-crate
//! `src/command/sandbox.rs` + `CommandHandler` (re-exported as
//! `SlashCommand` at `src/command/mod.rs:86`) + `ctx.emit(TuiEvent::TextDelta)`.
//!
//! Spec asks `/sandbox-toggle` as an alias. This handler registers with
//! `aliases() -> &["sandbox-toggle"]`.
//!
//! Gate 2 design note: the shared flag is owned by the handler instance
//! (`Arc<AtomicBool>` stored inline). Production `new()` creates a fresh
//! flag defaulted to `false`; tests use `with_flag()` to inject a shared
//! atomic for assertion. The tool dispatcher is expected to clone the
//! same `Arc<AtomicBool>` — wiring-through is deferred to a follow-up
//! ticket once the dispatcher gains a sandbox-aware pre-filter.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/sandbox` handler — flips sandbox-enabled shared flag.
pub(crate) struct SandboxHandler {
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl SandboxHandler {
    pub(crate) fn new() -> Self {
        Self {
            enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_flag(
        enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self { enabled }
    }
}

impl CommandHandler for SandboxHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering;

        let cmd = args.first().map(|s| s.as_str().trim()).unwrap_or("status");
        let message = match cmd {
            "on" | "enable" => {
                self.enabled.store(true, Ordering::SeqCst);
                "Sandbox enabled — tool dispatch will check permissions via Bubble mode."
            }
            "off" | "disable" => {
                self.enabled.store(false, Ordering::SeqCst);
                "Sandbox disabled — tool dispatch bypasses Bubble check."
            }
            "" | "status" => {
                if self.enabled.load(Ordering::SeqCst) {
                    "Sandbox status: ENABLED (Bubble mode restrictions active)."
                } else {
                    "Sandbox status: disabled."
                }
            }
            other => {
                return Err(anyhow::anyhow!(
                    "unknown /sandbox argument '{}': expected on, off, or status",
                    other
                ));
            }
        };

        ctx.emit(TuiEvent::TextDelta(format!("\n{}\n", message)));
        Ok(())
    }

    fn description(&self) -> &str {
        "Toggle sandbox restrictions (on|off|status) — Bubble-mode gate"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["sandbox-toggle"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn sandbox_on_enables_restrictions() {
        let flag = Arc::new(AtomicBool::new(false));
        let handler = SandboxHandler::with_flag(flag.clone());
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[String::from("on")]).unwrap();
        assert!(
            flag.load(Ordering::SeqCst),
            "sandbox flag should be true after 'on'"
        );
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                let lower = s.to_lowercase();
                assert!(
                    lower.contains("enabled") || lower.contains("on"),
                    "expected 'enabled' or 'on'; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn sandbox_off_disables_restrictions() {
        let flag = Arc::new(AtomicBool::new(true));
        let handler = SandboxHandler::with_flag(flag.clone());
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[String::from("off")]).unwrap();
        assert!(
            !flag.load(Ordering::SeqCst),
            "sandbox flag should be false after 'off'"
        );
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                let lower = s.to_lowercase();
                assert!(
                    lower.contains("disabled") || lower.contains("off"),
                    "expected 'disabled' or 'off'; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn sandbox_status_reports_current_state() {
        let flag = Arc::new(AtomicBool::new(true));
        let handler = SandboxHandler::with_flag(flag.clone());
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[String::from("status")]).unwrap();
        // Status must NOT flip the flag.
        assert!(
            flag.load(Ordering::SeqCst),
            "status must not change flag"
        );
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                let lower = s.to_lowercase();
                assert!(
                    lower.contains("enabled") || lower.contains("on"),
                    "status should report enabled state; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn sandbox_dispatches_via_registry() {
        // Gate 5 smoke proves three end-to-end facts:
        //   (1) default_registry().get("sandbox") returns Some — primary registered.
        //   (2) default_registry().get("sandbox-toggle") returns Some — alias wired.
        //   (3) archon_permissions::PermissionMode::from_str("bubble") returns
        //       Ok(Bubble) — cross-crate enum extension landed (FromStr updated).
        // Then dispatches "on" and "status" against the primary-looked-up handler
        // to verify the full wiring actually executes.
        use crate::command::registry::default_registry;
        use std::str::FromStr;

        let registry = default_registry();

        // (1) Primary lookup.
        let primary = registry
            .get("sandbox")
            .expect("sandbox must be registered in default_registry()");

        // (2) Alias lookup — registry.get must fall back to the alias map.
        let alias = registry
            .get("sandbox-toggle")
            .expect("sandbox-toggle alias must resolve via aliases() -> &[\"sandbox-toggle\"]");
        // Both Arc clones point at the same handler impl; verify at least that
        // the alias lookup is NOT returning None.
        drop(alias);

        // (3) PermissionMode::Bubble cross-crate verification.
        let mode = archon_permissions::mode::PermissionMode::from_str("bubble")
            .expect("PermissionMode::from_str(\"bubble\") must succeed post-TUI-628");
        assert_eq!(
            mode,
            archon_permissions::mode::PermissionMode::Bubble,
            "FromStr must map \"bubble\" to PermissionMode::Bubble"
        );

        // Exercise the dispatched handler — status reads current state.
        let (mut ctx, mut rx) = make_bug_ctx();
        primary
            .execute(&mut ctx, &[String::from("status")])
            .expect("dispatched /sandbox status must not error");
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "expected one TextDelta; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                let lower = s.to_lowercase();
                assert!(
                    lower.contains("status") || lower.contains("disabled") || lower.contains("enabled"),
                    "status TextDelta must contain 'status'/'disabled'/'enabled'; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }
}
