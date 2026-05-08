use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBackendKind {
    Disabled,
    Logical,
    Docker,
    Ssh,
    OpenShell,
}

impl SandboxBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Logical => "logical",
            Self::Docker => "docker",
            Self::Ssh => "ssh",
            Self::OpenShell => "openshell",
        }
    }

    pub fn is_real_isolation(self) -> bool {
        matches!(self, Self::Docker | Self::Ssh | Self::OpenShell)
    }
}

impl fmt::Display for SandboxBackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SandboxBackendKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "logical" => Ok(Self::Logical),
            "docker" => Ok(Self::Docker),
            "ssh" => Ok(Self::Ssh),
            "openshell" => Ok(Self::OpenShell),
            other => Err(format!(
                "sandbox.backend must be disabled, logical, docker, ssh, or openshell, got \"{other}\""
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    pub backend: SandboxBackendKind,
    pub mode: String,
    pub scope: String,
    pub workspace_access: String,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            backend: SandboxBackendKind::Disabled,
            mode: "risky".into(),
            scope: "session".into(),
            workspace_access: "ro".into(),
        }
    }
}

impl SandboxPolicy {
    pub fn validate(&self) -> Result<(), String> {
        match self.mode.as_str() {
            "risky" | "all" | "shell" => {}
            other => {
                return Err(format!(
                    "sandbox.mode must be risky, all, or shell, got \"{other}\""
                ));
            }
        }
        match self.scope.as_str() {
            "session" | "turn" | "tool" => {}
            other => {
                return Err(format!(
                    "sandbox.scope must be session, turn, or tool, got \"{other}\""
                ));
            }
        }
        match self.workspace_access.as_str() {
            "ro" | "rw" | "scratch" => Ok(()),
            other => Err(format!(
                "sandbox.workspace_access must be ro, rw, or scratch, got \"{other}\""
            )),
        }
    }

    pub fn describes_isolation(&self) -> &'static str {
        if self.backend.is_real_isolation() {
            "process isolation backend"
        } else if self.backend == SandboxBackendKind::Logical {
            "logical policy gate only"
        } else {
            "disabled"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_kind_parses_known_values() {
        assert_eq!(
            "openshell".parse::<SandboxBackendKind>().unwrap(),
            SandboxBackendKind::OpenShell
        );
        assert_eq!(
            SandboxBackendKind::Docker.to_string(),
            SandboxBackendKind::Docker.as_str()
        );
        assert!(SandboxBackendKind::Docker.is_real_isolation());
        assert!(!SandboxBackendKind::Logical.is_real_isolation());
    }

    #[test]
    fn policy_validation_rejects_unknown_workspace_access() {
        let policy = SandboxPolicy {
            workspace_access: "home".into(),
            ..SandboxPolicy::default()
        };

        let error = policy.validate().unwrap_err();

        assert!(error.contains("workspace_access"));
    }
}
