//! TASK-TUI-628 /sandbox slash-command handler.
//!
//! `/sandbox [on|off|status|explain]` — flips a shared `Arc<AtomicBool>` that
//! the tool-dispatch layer consults before invoking tool execution.
//! No args (or `status`) reads the current state without flipping.
//!
//! GHOST-006: flag is now shared between this handler (via CommandContext)
//! and the SandboxBackend impl that gates both dispatch paths. Toggling
//! via `/sandbox on/off` takes effect immediately on the next tool call.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::command::sandbox_doctor;

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
        let verbose = args.iter().any(|arg| arg == "--verbose" || arg == "-v");
        let message = match cmd {
            "on" | "enable" => {
                flag.store(true, Ordering::SeqCst);
                render_sandbox_status(true, verbose)
            }
            "off" | "disable" => {
                flag.store(false, Ordering::SeqCst);
                render_sandbox_status(false, verbose)
            }
            "" | "status" => render_sandbox_status(flag.load(Ordering::SeqCst), verbose),
            "explain" => render_sandbox_explain(flag.load(Ordering::SeqCst)),
            "doctor" => sandbox_doctor::render_sandbox_doctor(
                &args[1..],
                sandbox_doctor::SandboxDoctorOverrides::default(),
            ),
            other => {
                return Err(anyhow::anyhow!(
                    "unknown /sandbox argument '{}': expected on, off, status, explain, or doctor",
                    other
                ));
            }
        };

        ctx.emit(TuiEvent::TextDelta(format!("\n{message}\n")));
        Ok(())
    }

    fn description(&self) -> &str {
        "Toggle sandbox mode (on|off|status) — blocks write/shell/network tools when on"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["sandbox-toggle"]
    }
}

fn render_sandbox_status(enabled: bool, verbose: bool) -> String {
    let state = if enabled { "ON" } else { "OFF" };
    let effect = if enabled {
        "write, shell, network, and agent-spawn tools are blocked before execution"
    } else {
        "the sandbox backend is disabled; normal permission rules still apply"
    };
    let mut out = format!("Sandbox: logical gate {state} - {effect}.");
    if verbose {
        out.push_str(
            "\nBackend: logical policy gate\nIsolation: not OS/container isolation \
             (no container, chroot, seccomp, or VM boundary).",
        );
    }
    out
}

fn render_sandbox_explain(enabled: bool) -> String {
    let state = if enabled { "ON" } else { "OFF" };
    let routing = if enabled {
        "write/edit, shell, network, and agent-spawn tools are denied before execution"
    } else {
        "no sandbox backend denies tools; permission preflight and tool policy still apply"
    };
    format!(
        "Sandbox explain\n\
         Backend: logical\n\
         State: {state}\n\
         Isolation: policy gate only, not OS/container isolation\n\
         Routing: {routing}\n\
         Permissions: /sandbox off does not bypass permission modes, always_deny rules, or preflight checks\n\
         Future backends: Docker, SSH, and OpenShell are separate isolation backends and are not active here"
    )
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
                    s.contains("logical gate OFF"),
                    "expected OFF in message; got: {}",
                    s
                );
                assert!(
                    s.contains("normal permission rules still apply"),
                    "off message must not imply permission bypass; got: {}",
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
    fn sandbox_verbose_status_labels_logical_not_os_isolation() {
        let flag = Arc::new(AtomicBool::new(true));
        let handler = SandboxHandler::new();
        let (mut ctx, mut rx) = make_bug_ctx();
        ctx.sandbox_flag = Some(flag);
        handler
            .execute(
                &mut ctx,
                &[String::from("status"), String::from("--verbose")],
            )
            .unwrap();

        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(s.contains("Backend: logical"));
                assert!(s.contains("not OS/container isolation"));
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn sandbox_explain_says_off_does_not_bypass_permissions() {
        let flag = Arc::new(AtomicBool::new(false));
        let handler = SandboxHandler::new();
        let (mut ctx, mut rx) = make_bug_ctx();
        ctx.sandbox_flag = Some(flag);
        handler
            .execute(&mut ctx, &[String::from("explain")])
            .unwrap();

        let events = drain_tui_events(&mut rx);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(s.contains("Sandbox explain"));
                assert!(s.contains("State: OFF"));
                assert!(s.contains("does not bypass permission modes"));
                assert!(s.contains("Docker, SSH, and OpenShell"));
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
