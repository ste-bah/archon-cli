use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use archon_permissions::sandbox::{SandboxBackend, SandboxCommandRequest, SandboxCommandResult};

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
enum SandboxRouteMode {
    Risky,
    Shell,
    All,
}

impl SandboxRouteMode {
    fn from_config(value: &str) -> Self {
        match value {
            "all" => Self::All,
            "shell" => Self::Shell,
            _ => Self::Risky,
        }
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
    fn check(&self, tool: &str, input: &serde_json::Value) -> Result<(), String> {
        if self.mode.should_delegate_check(tool) {
            return self.inner.check(tool, input);
        }
        if matches!(tool, "PowerShell") {
            return Err(
                "sandbox mode routes shell execution through Bash-compatible backends; PowerShell cannot be sandbox-routed yet"
                    .into(),
            );
        }
        Ok(())
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

    #[derive(Debug)]
    struct DenyUnsupportedBackend;

    impl SandboxBackend for DenyUnsupportedBackend {
        fn check(&self, tool: &str, _input: &serde_json::Value) -> Result<(), String> {
            match tool {
                "Bash" | "Shell" | "Read" => Ok(()),
                other => Err(format!("blocked by real backend: {other}")),
            }
        }

        fn execute_bash<'a>(
            &'a self,
            _request: SandboxCommandRequest,
        ) -> Pin<Box<dyn Future<Output = Option<SandboxCommandResult>> + Send + 'a>> {
            Box::pin(async {
                Some(SandboxCommandResult {
                    content: "sandboxed".into(),
                    is_error: false,
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

        assert!(backend.check("Bash", &serde_json::json!({})).is_ok());
        assert!(backend.check("Write", &serde_json::json!({})).is_ok());
        assert!(backend.check("Edit", &serde_json::json!({})).is_ok());
        assert!(backend.check("WebFetch", &serde_json::json!({})).is_ok());
    }

    #[test]
    fn all_mode_keeps_strict_backend_compatibility() {
        let backend =
            apply_configured_sandbox_mode(Arc::new(DenyUnsupportedBackend), &docker_config("all"));

        let error = backend.check("Write", &serde_json::json!({})).unwrap_err();

        assert!(error.contains("blocked by real backend"));
    }

    #[test]
    fn shell_mode_does_not_allow_unsandboxed_powershell() {
        let backend = apply_configured_sandbox_mode(
            Arc::new(DenyUnsupportedBackend),
            &docker_config("shell"),
        );

        let error = backend
            .check("PowerShell", &serde_json::json!({}))
            .unwrap_err();

        assert!(error.contains("PowerShell cannot be sandbox-routed yet"));
    }
}
