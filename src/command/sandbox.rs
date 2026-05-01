//! TASK-TUI-628 /sandbox slash-command handler.
//!
//! `/sandbox [on|off|status]` — flips a shared `Arc<AtomicBool>` that
//! the tool-dispatch layer consults before invoking tool execution.
//! No args (or `status`) reads the current state without flipping.
//!
//! GHOST-006: flag is now shared between this handler (via CommandContext)
//! and the SandboxBackend impl that gates both dispatch paths. Toggling
//! via `/sandbox on/off` takes effect immediately on the next tool call.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// `/sandbox` handler — flips sandbox-enabled shared flag via CommandContext.
pub(crate) struct SandboxHandler;

impl SandboxHandler {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl CommandHandler for SandboxHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering;

        let flag = ctx.sandbox_flag.as_ref().ok_or_else(|| {
            anyhow::anyhow!("sandbox flag not initialised — session boot wiring gap")
        })?;

        let cmd = args.first().map(|s| s.as_str().trim()).unwrap_or("status");
        let message = match cmd {
            "on" | "enable" => {
                flag.store(true, Ordering::SeqCst);
                "Sandbox: ON — write, shell, network, and agent-spawn tools are now blocked."
            }
            "off" | "disable" => {
                flag.store(false, Ordering::SeqCst);
                "Sandbox: OFF — all tools permitted."
            }
            "" | "status" => {
                if flag.load(Ordering::SeqCst) {
                    "Sandbox: ON (write, shell, network, and agent-spawn tools blocked)."
                } else {
                    "Sandbox: OFF (all tools permitted)."
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
        "Toggle sandbox mode (on|off|status) — blocks write/shell/network tools when on"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["sandbox-toggle"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn sandbox_on_enables_restrictions() {
        let flag = Arc::new(AtomicBool::new(false));
        let handler = SandboxHandler::new();
        let (mut ctx, mut rx) = make_bug_ctx();
        ctx.sandbox_flag = Some(flag.clone());
        handler.execute(&mut ctx, &[String::from("on")]).unwrap();
        assert!(
            flag.load(Ordering::SeqCst),
            "sandbox flag should be true after 'on'"
        );
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.to_lowercase().contains("on"),
                    "expected 'on' in message; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn sandbox_off_disables_restrictions() {
        let flag = Arc::new(AtomicBool::new(true));
        let handler = SandboxHandler::new();
        let (mut ctx, mut rx) = make_bug_ctx();
        ctx.sandbox_flag = Some(flag.clone());
        handler.execute(&mut ctx, &[String::from("off")]).unwrap();
        assert!(
            !flag.load(Ordering::SeqCst),
            "sandbox flag should be false after 'off'"
        );
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.to_lowercase().contains("off"),
                    "expected 'off' in message; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn sandbox_status_reports_current_state() {
        let flag = Arc::new(AtomicBool::new(true));
        let handler = SandboxHandler::new();
        let (mut ctx, mut rx) = make_bug_ctx();
        ctx.sandbox_flag = Some(flag.clone());
        handler
            .execute(&mut ctx, &[String::from("status")])
            .unwrap();
        assert!(flag.load(Ordering::SeqCst), "status must not change flag");
        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.to_lowercase().contains("on"),
                    "status should report on when flag is true; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn sandbox_missing_flag_errors() {
        let handler = SandboxHandler::new();
        let (mut ctx, _rx) = make_bug_ctx();
        let result = handler.execute(&mut ctx, &[String::from("on")]);
        assert!(result.is_err(), "should fail when sandbox_flag is None");
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn sandbox_dispatches_via_registry() {
        use crate::command::registry::default_registry;
        use std::str::FromStr;

        let registry = default_registry();

        let primary = registry
            .get("sandbox")
            .expect("sandbox must be registered in default_registry()");

        let alias = registry
            .get("sandbox-toggle")
            .expect("sandbox-toggle alias must resolve via aliases()");
        drop(alias);

        let mode = archon_permissions::mode::PermissionMode::from_str("bubble")
            .expect("PermissionMode::from_str(\"bubble\") must succeed");
        assert_eq!(
            mode,
            archon_permissions::mode::PermissionMode::Bubble,
            "FromStr must map \"bubble\" to PermissionMode::Bubble"
        );

        let (mut ctx, mut rx) = make_bug_ctx();
        ctx.sandbox_flag = Some(Arc::new(AtomicBool::new(false)));
        primary
            .execute(&mut ctx, &[String::from("status")])
            .expect("dispatched /sandbox status must not error");
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1, "expected one TextDelta; got: {:?}", events);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.contains("OFF"),
                    "status TextDelta must contain OFF; got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }
}
