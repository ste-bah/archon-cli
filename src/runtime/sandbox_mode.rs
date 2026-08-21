use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use archon_permissions::sandbox::{
    SandboxBackend, SandboxCommandRequest, SandboxCommandResult, SandboxTerminal,
    SandboxTerminalRequest,
};

pub(crate) fn apply_configured_sandbox_mode(
    inner: Arc<dyn SandboxBackend>,
    config: &archon_core::sandbox::SandboxConfig,
) -> Arc<dyn SandboxBackend> {
    let Ok(kind) = config.backend_kind() else {
        return inner;
    };
    if !kind.is_real_isolation() {
        return inner;
    }
    Arc::new(ModeScopedSandboxBackend {
        inner,
        mode: SandboxRouteMode::from_config(&config.mode),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SandboxRouteMode {
    Risky,
    Shell,
    All,
}

/// What the configured mode does with one tool before any backend sees it.
///
/// Named rather than inlined into `ModeScopedSandboxBackend::check` because
/// `archon sandbox explain` has to say which of these three happened: "the
/// backend allowed it" and "the backend was never asked" are both `Ok(())`
/// here, and reporting them as the same thing is how an explanation starts
/// describing a decision nobody makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModeRouting {
    /// The mode defers to the backend, which decides on the declared class.
    Backend,
    /// The mode itself refuses, without consulting the backend.
    RefusedByMode(&'static str),
    /// The mode does not defer this tool, so the normal permission preflight
    /// is what decides it.
    PermissionPreflight,
}

/// A shell Archon cannot route into a Bash-compatible backend, and so will not
/// run under one at all.
const POWERSHELL_NOT_ROUTABLE: &str = "sandbox mode routes shell execution through Bash-compatible backends; PowerShell cannot be \
     sandbox-routed yet";

impl SandboxRouteMode {
    pub(crate) fn from_config(value: &str) -> Self {
        match value {
            "all" => Self::All,
            "shell" => Self::Shell,
            _ => Self::Risky,
        }
    }

    pub(crate) fn route(self, tool: &str) -> ModeRouting {
        if self.should_delegate_check(tool) {
            return ModeRouting::Backend;
        }
        if matches!(tool, "PowerShell") {
            return ModeRouting::RefusedByMode(POWERSHELL_NOT_ROUTABLE);
        }
        ModeRouting::PermissionPreflight
    }

    /// Whether the mode's shell list names this tool, whatever the configured
    /// mode is.
    ///
    /// `Shell` is the mode whose delegation is exactly that list, so asking it
    /// is asking the list itself — and the list stays in one place. `explain`
    /// needs this to report the case where the list names something no tool
    /// declares: `Shell` itself, which would be routed here and resolves to
    /// nothing in the registry.
    pub(crate) fn is_named_shell_tool(tool: &str) -> bool {
        matches!(Self::Shell.route(tool), ModeRouting::Backend)
    }

    fn should_delegate_check(self, tool: &str) -> bool {
        match self {
            Self::All => true,
            Self::Risky | Self::Shell => matches!(tool, "Bash" | "Shell"),
        }
    }
}

#[derive(Debug)]
struct ModeScopedSandboxBackend {
    inner: Arc<dyn SandboxBackend>,
    mode: SandboxRouteMode,
}

impl SandboxBackend for ModeScopedSandboxBackend {
    fn check(
        &self,
        tool: &str,
        capability: archon_permissions::ToolCapability,
        input: &serde_json::Value,
    ) -> Result<(), String> {
        match self.mode.route(tool) {
            ModeRouting::Backend => self.inner.check(tool, capability, input),
            ModeRouting::RefusedByMode(reason) => Err(reason.into()),
            ModeRouting::PermissionPreflight => Ok(()),
        }
    }

    /// Delegated whatever the mode is, exactly as `execute_bash` is.
    ///
    /// `mode` scopes which tools have their *permission* deferred to the
    /// backend; it has never scoped where execution happens. A terminal is
    /// execution, so under every mode it belongs in the backend's world.
    fn terminal(&self, request: &SandboxTerminalRequest) -> SandboxTerminal {
        self.inner.terminal(request)
    }

    fn execute_bash<'a>(
        &'a self,
        request: SandboxCommandRequest,
    ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
        self.inner.execute_bash(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_permissions::ToolCapability;

    #[derive(Debug)]
    struct DenyUnsupportedBackend;

    impl SandboxBackend for DenyUnsupportedBackend {
        fn check(
            &self,
            tool: &str,
            _capability: archon_permissions::ToolCapability,
            _input: &serde_json::Value,
        ) -> Result<(), String> {
            match tool {
                "Bash" | "Shell" | "Read" => Ok(()),
                other => Err(format!("blocked by real backend: {other}")),
            }
        }

        fn terminal(&self, _request: &SandboxTerminalRequest) -> SandboxTerminal {
            SandboxTerminal::Open(archon_permissions::SandboxTerminalCommand {
                program: "fake-backend".into(),
                args: vec!["--shell".into()],
                shell: "bash".into(),
                location: "/workspace in the fake world".into(),
            })
        }

        fn execute_bash<'a>(
            &'a self,
            _request: SandboxCommandRequest,
        ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
            Box::pin(async {
                Some(SandboxCommandResult {
                    content: "sandboxed".into(),
                    is_error: false,
                    exit_code: Some(0),
                })
            })
        }
    }

    fn docker_config(mode: &str) -> archon_core::sandbox::SandboxConfig {
        archon_core::sandbox::SandboxConfig {
            backend: "docker".into(),
            mode: mode.into(),
            ..archon_core::sandbox::SandboxConfig::default()
        }
    }

    #[test]
    fn risky_mode_routes_shell_but_leaves_host_edit_tools_to_permissions() {
        let backend = apply_configured_sandbox_mode(
            Arc::new(DenyUnsupportedBackend),
            &docker_config("risky"),
        );

        assert!(
            backend
                .check("Bash", ToolCapability::EXECUTION, &serde_json::json!({}))
                .is_ok()
        );
        assert!(
            backend
                .check("Write", ToolCapability::FILE_WRITE, &serde_json::json!({}))
                .is_ok()
        );
        assert!(
            backend
                .check("Edit", ToolCapability::FILE_WRITE, &serde_json::json!({}))
                .is_ok()
        );
        assert!(
            backend
                .check("WebFetch", ToolCapability::Egress, &serde_json::json!({}))
                .is_ok()
        );
    }

    #[test]
    fn all_mode_keeps_strict_backend_compatibility() {
        let backend =
            apply_configured_sandbox_mode(Arc::new(DenyUnsupportedBackend), &docker_config("all"));

        let error = backend
            .check("Write", ToolCapability::FILE_WRITE, &serde_json::json!({}))
            .unwrap_err();

        assert!(error.contains("blocked by real backend"));
    }

    /// Under the default `risky` mode `check` is not consulted for terminal
    /// tools at all, so if the mode scoped `terminal` the way it scopes
    /// `check`, a terminal would open on the host while `Bash` went through the
    /// backend. That is the #201 Phase 6 bypass, restated as a test.
    #[test]
    fn every_mode_puts_a_terminal_in_the_backends_world() {
        for mode in ["risky", "shell", "all"] {
            let backend = apply_configured_sandbox_mode(
                Arc::new(DenyUnsupportedBackend),
                &docker_config(mode),
            );

            let terminal = backend.terminal(&SandboxTerminalRequest {
                shell: None,
                workspace: std::path::PathBuf::from("/repo"),
                cwd: std::path::PathBuf::from("/repo"),
            });

            assert!(
                matches!(terminal, SandboxTerminal::Open(_)),
                "{mode} mode let a terminal out onto the host"
            );
        }
    }

    #[test]
    fn shell_mode_does_not_allow_unsandboxed_powershell() {
        let backend = apply_configured_sandbox_mode(
            Arc::new(DenyUnsupportedBackend),
            &docker_config("shell"),
        );

        let error = backend
            .check(
                "PowerShell",
                ToolCapability::HOST_HANDLE,
                &serde_json::json!({}),
            )
            .unwrap_err();

        assert!(error.contains("PowerShell cannot be sandbox-routed yet"));
    }
}
