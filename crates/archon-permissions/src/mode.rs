use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Permission mode controlling how tool executions are gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    /// Prompt for risky/dangerous operations (legacy: Ask).
    Default,
    /// Auto-allow file edits (Read/Write/Edit/Glob/Grep), prompt for Bash.
    AcceptEdits,
    /// Read-only: only whitelisted tools allowed.
    Plan,
    /// Heuristic-based: auto-approve safe, prompt risky, warn dangerous.
    Auto,
    /// Auto-allow everything except always_deny rules.
    DontAsk,
    /// Skip all permission checks entirely (legacy: Yolo).
    BypassPermissions,
}

impl PermissionMode {
    /// Cycle to the next mode. When `allow_bypass` is false,
    /// `BypassPermissions` is skipped in the rotation.
    pub fn next_mode(&self, allow_bypass: bool) -> Self {
        match self {
            Self::Default => Self::AcceptEdits,
            Self::AcceptEdits => Self::Plan,
            Self::Plan => Self::Auto,
            Self::Auto => Self::DontAsk,
            Self::DontAsk => {
                if allow_bypass {
                    Self::BypassPermissions
                } else {
                    Self::Default
                }
            }
            Self::BypassPermissions => Self::Default,
        }
    }

    /// Canonical string name for this mode.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::Plan => "plan",
            Self::Auto => "auto",
            Self::DontAsk => "dontAsk",
            Self::BypassPermissions => "bypassPermissions",
        }
    }
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Default
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PermissionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            // Canonical names (case-sensitive)
            "default" => Ok(Self::Default),
            "acceptEdits" => Ok(Self::AcceptEdits),
            "plan" => Ok(Self::Plan),
            "auto" => Ok(Self::Auto),
            "dontAsk" => Ok(Self::DontAsk),
            "bypassPermissions" => Ok(Self::BypassPermissions),
            // Legacy aliases
            "ask" => Ok(Self::Default),
            "yolo" => Ok(Self::BypassPermissions),
            other => Err(format!(
                "unknown permission mode '{other}'; valid modes: \
                 default, acceptEdits, plan, auto, dontAsk, bypassPermissions \
                 (aliases: ask, yolo)"
            )),
        }
    }
}

/// The result of a permission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Tool execution is allowed.
    Allow,
    /// Tool execution requires user confirmation.
    NeedsPermission(String),
    /// Tool execution is denied.
    Deny(String),
}
