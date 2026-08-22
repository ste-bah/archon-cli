use archon_permissions::sandbox::SandboxScopeSupport;
use serde::{Deserialize, Serialize};

mod capability_gate;
mod docker;
mod openshell;
mod policy;
// Shared by the ssh and openshell `remote` modes: both hold the workspace on
// the far side of a command transport, and the commands that read and write it
// are the same commands. Only the transport differs.
mod remote_fs;
mod ssh;

pub use capability_gate::{CapabilityAllowance, check_capability};
pub use docker::{
    DockerConfig, DockerDoctorReport, DockerDoctorStatus, DockerFs, DockerProbe,
    DockerSandboxBackend, docker_doctor_report, probe_docker, render_docker_doctor_report,
};
pub use openshell::{
    OpenShellConfig, OpenShellDoctorReport, OpenShellDoctorStatus, OpenShellProbe,
    OpenShellSandboxBackend, openshell_doctor_report, openshell_filesystem, probe_openshell,
    render_openshell_doctor_report,
};
pub use policy::{SandboxBackendKind, SandboxPolicy};
pub use ssh::{
    SshConfig, SshDoctorReport, SshDoctorStatus, SshProbe, SshSandboxBackend, probe_ssh,
    render_ssh_doctor_report, ssh_doctor_report, ssh_filesystem,
};

// No container-level `#[serde(default)]`: it is a deserialization attribute and
// `Deserialize` is hand-written below, so leaving it here would only suggest
// this struct fills its own defaults when the impl is what actually does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SandboxConfig {
    pub backend: String,
    pub mode: String,
    pub scope: String,
    pub workspace_access: String,
    /// Whether `scope` came from the configuration or from this struct's
    /// default.
    ///
    /// Refusing to load is right for a lifetime an operator asked for and the
    /// backend cannot keep. It is not right for one they never chose: `scope`
    /// defaults to `session`, so `backend = "openshell"` with no `scope` key at
    /// all would have failed the *whole* configuration over a value nobody
    /// wrote. Not serialised — it describes where the value came from, not what
    /// it is, and a round trip through TOML would invent an answer.
    #[serde(skip)]
    pub scope_explicit: bool,
    pub docker: DockerConfig,
    pub ssh: SshConfig,
    pub openshell: OpenShellConfig,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: "disabled".into(),
            mode: "risky".into(),
            scope: "session".into(),
            scope_explicit: false,
            workspace_access: "ro".into(),
            docker: DockerConfig::default(),
            ssh: SshConfig::default(),
            openshell: OpenShellConfig::default(),
        }
    }
}

/// Hand-written so `scope_explicit` can record whether the key was present.
///
/// `#[serde(default)]` on the derive would fill `scope` in silently, which is
/// the whole distinction this needs to keep.
impl<'de> Deserialize<'de> for SandboxConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Every scalar is an `Option` so an absent key stays distinguishable
        /// from one written with the default value; the nested sections keep
        /// their own defaults, which already do the right thing.
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Wire {
            backend: Option<String>,
            mode: Option<String>,
            scope: Option<String>,
            workspace_access: Option<String>,
            docker: DockerConfig,
            ssh: SshConfig,
            openshell: OpenShellConfig,
        }

        let wire = Wire::deserialize(deserializer)?;
        let defaults = SandboxConfig::default();
        Ok(SandboxConfig {
            backend: wire.backend.unwrap_or(defaults.backend),
            mode: wire.mode.unwrap_or(defaults.mode),
            scope_explicit: wire.scope.is_some(),
            scope: wire.scope.unwrap_or(defaults.scope),
            workspace_access: wire.workspace_access.unwrap_or(defaults.workspace_access),
            docker: wire.docker,
            ssh: wire.ssh,
            openshell: wire.openshell,
        })
    }
}

/// What a configuration's `sandbox.scope` actually resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeDecision {
    /// The backend does what the configuration asked for.
    Honoured {
        scope: archon_permissions::SandboxScope,
        support: SandboxScopeSupport,
    },
    /// The backend cannot keep the scope, but nobody chose it — it is the
    /// struct default — so the narrowest lifetime is used instead and this says
    /// why. `tool` is the fallback because it is the least sharing a backend can
    /// do and the behaviour that predates any of this.
    FellBack {
        scope: archon_permissions::SandboxScope,
        from: archon_permissions::SandboxScope,
        reason: String,
    },
    /// The backend creates no world, so no scope names anything.
    NotApplicable,
}

impl ScopeDecision {
    /// The lifetime that will actually be used, if any.
    #[must_use]
    pub fn scope(&self) -> Option<archon_permissions::SandboxScope> {
        match self {
            Self::Honoured { scope, .. } | Self::FellBack { scope, .. } => Some(*scope),
            Self::NotApplicable => None,
        }
    }
}

impl SandboxConfig {
    pub fn validate(&self) -> Result<(), String> {
        let policy = self.policy()?;
        policy.validate()?;
        self.docker.validate()?;
        self.ssh.validate()?;
        self.openshell.validate()?;
        if let ScopeDecision::FellBack {
            scope,
            from,
            reason,
        } = self.scope_decision()?
        {
            tracing::warn!(
                configured = %from,
                effective = %scope,
                %reason,
                "sandbox.scope was not set and this backend cannot keep the default; \
                 using the narrowest lifetime instead"
            );
        }
        Ok(())
    }

    /// What the selected backend does with the selected `sandbox.scope`.
    ///
    /// The configuration asks the backend rather than deciding for it, which is
    /// the precedent `SandboxBackend::terminal` set: the backend is the only
    /// thing that knows whether it can hold a sandbox open, and one that cannot
    /// says so — at config load, in the operator's own vocabulary — instead of
    /// quietly doing something else at the first command.
    ///
    /// An explicitly chosen scope the backend cannot keep is an error. A
    /// defaulted one falls back to `tool`, because failing a whole configuration
    /// over a value the operator never wrote is a different thing from refusing
    /// one they did.
    pub fn scope_decision(&self) -> Result<ScopeDecision, String> {
        let scope = self.policy()?.scope_kind()?;
        let Some(backend) = self.bare_backend()? else {
            return Ok(ScopeDecision::NotApplicable);
        };
        let support = backend.scope_support(scope);
        let SandboxScopeSupport::Unsupported(reason) = support else {
            return Ok(ScopeDecision::Honoured { scope, support });
        };
        if self.scope_explicit {
            return Err(reason);
        }
        // Verified rather than assumed: `tool` is the narrowest lifetime there
        // is, but a backend that cannot keep even that has no honest fallback
        // and must say so rather than be given one.
        let fallback = archon_permissions::SandboxScope::Tool;
        backend.scope_support(fallback).into_result()?;
        Ok(ScopeDecision::FellBack {
            scope: fallback,
            from: scope,
            reason,
        })
    }

    /// The backend a configuration names, with no mode or audit wrapper on it.
    ///
    /// Wrappers delegate `scope_support`, but building them here would drag the
    /// binary's runtime layers into config validation; this only ever asks a
    /// question the wrappers forward verbatim.
    fn bare_backend(&self) -> Result<Option<Box<dyn archon_permissions::SandboxBackend>>, String> {
        Ok(match self.backend_kind()? {
            SandboxBackendKind::Disabled | SandboxBackendKind::Logical => None,
            SandboxBackendKind::Docker => Some(Box::new(DockerSandboxBackend::new(
                self.docker.clone(),
                self.workspace_access.clone(),
                self.policy()?.scope_kind()?,
            ))),
            SandboxBackendKind::Ssh => Some(Box::new(SshSandboxBackend::new(self.ssh.clone()))),
            SandboxBackendKind::OpenShell => Some(Box::new(OpenShellSandboxBackend::new(
                self.openshell.clone(),
            ))),
        })
    }

    pub fn backend_kind(&self) -> Result<SandboxBackendKind, String> {
        self.backend.parse()
    }

    pub fn policy(&self) -> Result<SandboxPolicy, String> {
        Ok(SandboxPolicy {
            backend: self.backend_kind()?,
            mode: self.mode.clone(),
            scope: self.scope.clone(),
            workspace_access: self.workspace_access.clone(),
        })
    }
}

/// The filesystem of the world this configuration executes in (#201 Phase 2).
///
/// `Ok(None)` means the host, and is the honest answer in more cases than it
/// looks: a disabled or `logical` backend never leaves the host, docker's
/// workspace is a bind mount of it, and both `mirror` modes assume the same
/// tree is visible on the far side. Only a genuinely remote workspace needs a
/// filesystem of its own.
///
/// An error here must fail session boot rather than degrade to the host. The
/// whole point is that the agent's reads and its shell agree about which tree
/// they are on; quietly handing back the host when the remote filesystem could
/// not be built would restore exactly the split this issue exists to close,
/// and would do it silently.
pub fn sandbox_filesystem(
    config: &SandboxConfig,
    working_dir: &std::path::Path,
) -> Result<Option<std::sync::Arc<dyn archon_tools::filesystem::FileSystem>>, String> {
    match config.backend_kind()? {
        SandboxBackendKind::Disabled | SandboxBackendKind::Logical => Ok(None),
        SandboxBackendKind::Docker => Ok(Some(std::sync::Arc::new(DockerFs::new(working_dir)))),
        SandboxBackendKind::Ssh => ssh_filesystem(&config.ssh, working_dir).map(Some),
        SandboxBackendKind::OpenShell => {
            openshell_filesystem(&config.openshell, working_dir).map(Some)
        }
    }
}

pub(crate) fn first_non_empty_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
