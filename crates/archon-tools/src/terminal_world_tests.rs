//! Where a terminal ends up, per backend answer (#201 Phase 6).
//!
//! The assertion that matters is the negative one: with a backend that holds
//! the execution world, nothing here may produce a host shell. Everything else
//! is about the answer being *specific* enough for the model to act on.

use std::ffi::OsString;
use std::sync::Arc;

use archon_permissions::sandbox::{SandboxBackend, SandboxTerminalCommand};

use super::*;

/// A backend that returns whatever answer a test needs.
///
/// `pub(crate)` so `terminal_tools_tests` drives the tools through the same
/// fake rather than keeping a second copy that could drift from this one.
#[derive(Debug)]
pub(crate) struct FixedTerminalBackend {
    answer: SandboxTerminal,
}

impl FixedTerminalBackend {
    pub(crate) fn refusing(reason: &str) -> Arc<Self> {
        Arc::new(Self {
            answer: SandboxTerminal::Refused(reason.to_string()),
        })
    }

    pub(crate) fn host() -> Arc<Self> {
        Arc::new(Self {
            answer: SandboxTerminal::Host,
        })
    }

    pub(crate) fn opening(command: SandboxTerminalCommand) -> Arc<Self> {
        Arc::new(Self {
            answer: SandboxTerminal::Open(command),
        })
    }
}

impl SandboxBackend for FixedTerminalBackend {
    /// Deliberately permissive, and that is the point: under the default
    /// `sandbox.mode = "risky"` nothing consults `check` for terminal tools, so
    /// a plan that relied on it would pass this file while shipping the bypass.
    fn check(&self, _tool: &str, _input: &serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    fn terminal(&self, _request: &SandboxTerminalRequest) -> SandboxTerminal {
        self.answer.clone()
    }
}

pub(crate) fn door() -> SandboxTerminalCommand {
    SandboxTerminalCommand {
        program: "container-door".into(),
        args: vec!["run".into(), "--tty".into(), "/bin/bash".into()],
        shell: "bash".into(),
        location: "/workspace in the ubuntu:24.04 container".into(),
    }
}

fn ctx(sandbox: Option<Arc<dyn SandboxBackend>>) -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "terminal-world-tests".into(),
        sandbox,
        ..ToolContext::default()
    }
}

fn argv(launch: &Launch) -> Vec<OsString> {
    launch.command.get_argv().to_vec()
}

#[test]
fn without_a_backend_a_terminal_is_a_host_shell() {
    let ctx = ctx(None);

    let launch = plan(&ctx, None, &ctx.working_dir).expect("no sandbox, no obstacle");

    assert_eq!(launch.shell, shells::default_shell());
    assert!(!launch.sandboxed);
    assert_eq!(launch.location, ctx.working_dir.display().to_string());
}

/// The bug this phase exists to remove. A backend that holds the execution
/// world and cannot host a shell must produce an error, never a host PTY.
#[test]
fn a_backend_that_cannot_host_a_shell_refuses_instead_of_spawning_one() {
    let ctx = ctx(Some(FixedTerminalBackend::refusing(
        "openshell sandbox: throwaway sandbox per command",
    )));

    let error = plan(&ctx, None, &ctx.working_dir).expect_err("must not fall back to the host");

    assert!(error.contains("throwaway sandbox per command"), "{error}");
}

#[test]
fn an_opening_backend_supplies_the_command_that_is_actually_run() {
    let ctx = ctx(Some(FixedTerminalBackend::opening(door())));

    let launch = plan(&ctx, None, &ctx.working_dir).expect("the backend opened one");

    assert!(launch.sandboxed);
    assert_eq!(launch.shell, "bash");
    assert_eq!(launch.location, "/workspace in the ubuntu:24.04 container");
    assert_eq!(
        argv(&launch),
        vec![
            OsString::from("container-door"),
            OsString::from("run"),
            OsString::from("--tty"),
            OsString::from("/bin/bash"),
        ],
        "the launched process must be the backend's door, not a host shell"
    );
}

/// A policy-only backend — `/sandbox on` with no isolation configured — denies
/// tools without relocating them. Refusing its terminals would break a feature
/// for no gain, so it says so explicitly and gets a host shell.
#[test]
fn a_policy_only_backend_still_gets_a_host_shell() {
    let ctx = ctx(Some(FixedTerminalBackend::host()));

    let launch = plan(&ctx, Some("sh"), &ctx.working_dir).expect("host is a valid answer");

    assert!(!launch.sandboxed);
    assert_eq!(launch.shell, "sh");
}

#[test]
fn an_unknown_shell_is_still_refused_on_the_host_path() {
    let ctx = ctx(None);

    let error = plan(&ctx, Some("fish"), &ctx.working_dir).expect_err("fish is not offered");

    assert!(error.contains("fish"), "{error}");
}

#[test]
fn host_terminals_are_allowed_only_when_no_backend_claims_the_world() {
    assert!(host_terminals_allowed(&ctx(None)));
    assert!(host_terminals_allowed(&ctx(Some(
        FixedTerminalBackend::host()
    ))));
    assert!(
        !host_terminals_allowed(&ctx(Some(FixedTerminalBackend::opening(door())))),
        "a backend that opens its own terminals owns the execution world"
    );
    assert!(
        !host_terminals_allowed(&ctx(Some(FixedTerminalBackend::refusing("no session")))),
        "a backend that refuses a terminal has not licensed a host one instead"
    );
}

/// The caller's `cwd` reaches the backend unaltered, because that is the only
/// thing that lets a backend refuse a directory its world cannot reach.
#[test]
fn the_request_carries_the_workspace_and_the_requested_directory_apart() {
    #[derive(Debug)]
    struct Capturing(std::sync::Mutex<Option<SandboxTerminalRequest>>);

    impl SandboxBackend for Capturing {
        fn check(&self, _tool: &str, _input: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }

        fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
            *self.0.lock().expect("lock") = Some(request.clone());
            SandboxTerminal::Open(door())
        }
    }

    let backend = Arc::new(Capturing(std::sync::Mutex::new(None)));
    let ctx = ctx(Some(backend.clone()));
    let sub = ctx.working_dir.join("crates");

    plan(&ctx, Some("bash"), &sub).expect("opened");

    let seen = backend.0.lock().expect("lock").clone().expect("captured");
    assert_eq!(seen.shell, Some("bash".into()));
    assert_eq!(seen.workspace, ctx.working_dir);
    assert_eq!(seen.cwd, sub);
}
