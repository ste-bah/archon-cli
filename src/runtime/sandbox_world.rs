//! The execution world a sandbox configuration names (#201 Phase 4).
//!
//! The session path built its backend inline, which meant the workflow CLI —
//! which composes its own `AgentConfig` and `ToolContext` in
//! `command::pipeline_support` rather than going through session boot — had no
//! backend at all. `sandbox.backend = "docker"` configured a session and left
//! every `w.agent()` call running straight on the host, so the issue's
//! acceptance criterion ("a workflow completes under `backend = docker`") could
//! not be met however correct the layers underneath were.
//!
//! One constructor, so the two entry points cannot come to disagree about what
//! a configuration means — which is the same failure mode #201 opens with,
//! three near-identical allowlists drifting apart.

use std::sync::Arc;

use archon_core::sandbox::SandboxConfig;
use archon_permissions::sandbox::SandboxBackend;

/// The backend for a configuration that isolates execution, mode already
/// applied.
///
/// `None` means this configuration does not isolate — `disabled` or the
/// policy-only `logical` backend — and the caller supplies whatever host-side
/// policy backend it uses for that case. Returning `None` rather than a host
/// backend keeps the choice with the caller: the session has a `/sandbox`
/// toggle to honour and the workflow CLI has none.
pub(crate) fn isolation_backend(config: &SandboxConfig) -> Option<Arc<dyn SandboxBackend>> {
    if !config.backend_kind().ok()?.is_real_isolation() {
        return None;
    }
    let backend: Arc<dyn SandboxBackend> = match config.backend.as_str() {
        "docker" => Arc::new(archon_core::sandbox::DockerSandboxBackend::new(
            config.docker.clone(),
            config.workspace_access.clone(),
        )),
        "ssh" => Arc::new(archon_core::sandbox::SshSandboxBackend::new(
            config.ssh.clone(),
        )),
        "openshell" => Arc::new(archon_core::sandbox::OpenShellSandboxBackend::new(
            config.openshell.clone(),
        )),
        // `is_real_isolation` already excluded everything else, so this is
        // unreachable rather than a fallback worth designing.
        _ => return None,
    };
    Some(super::sandbox_mode::apply_configured_sandbox_mode(
        backend, config,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_permissions::ToolCapability;

    fn config(backend: &str) -> SandboxConfig {
        SandboxConfig {
            backend: backend.into(),
            // `all`, so `check` is actually delegated to the backend rather
            // than short-circuited by the default `risky` routing. Under
            // `risky` every backend answers `Ok` for a non-shell tool, which
            // would make the assertions below true of a host backend too.
            mode: "all".into(),
            docker: archon_core::sandbox::DockerConfig {
                enabled: true,
                ..archon_core::sandbox::DockerConfig::default()
            },
            ..SandboxConfig::default()
        }
    }

    #[test]
    fn an_isolating_configuration_produces_a_backend_that_gates() {
        let backend = isolation_backend(&config("docker")).expect("docker isolates execution");

        backend
            .check(
                "Agent",
                ToolCapability::ControlPlane,
                &serde_json::json!({}),
            )
            .expect("Phase 4 opened spawning");
        assert!(
            backend
                .check("lsp", ToolCapability::HOST_HANDLE, &serde_json::json!({}))
                .is_err(),
            "a host handle escaped the container"
        );
    }

    /// The caller owns the host case. Handing back a backend here would take
    /// the `/sandbox` toggle away from the session without saying so.
    #[test]
    fn a_non_isolating_configuration_has_no_backend_of_its_own() {
        for backend in ["disabled", "logical", "nonsense"] {
            assert!(
                isolation_backend(&config(backend)).is_none(),
                "{backend} should leave the backend to the caller"
            );
        }
    }
}
