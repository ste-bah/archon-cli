use std::str::FromStr;

use archon_permissions::mode::PermissionMode;
use archon_tools::tool::AgentMode;

use crate::agents::definition::PermissionMode as DefinitionPermissionMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionOverlayReason {
    NoRequest,
    Applied,
    ParentModeLocked,
    BlockedExpansion,
    BlockedDangerousBypass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionOverlayDecision {
    pub parent_mode: PermissionMode,
    pub requested_mode: Option<PermissionMode>,
    pub effective_mode: PermissionMode,
    pub reason: PermissionOverlayReason,
}

impl PermissionOverlayDecision {
    pub fn agent_mode(self) -> AgentMode {
        AgentMode::from(&self.effective_mode)
    }
}

pub fn resolve_permission_overlay(
    parent_mode: &str,
    requested_mode: Option<&DefinitionPermissionMode>,
    allow_bypass: bool,
) -> PermissionOverlayDecision {
    let parent_mode = PermissionMode::from_str(parent_mode).unwrap_or_default();
    let requested_mode = requested_mode.map(definition_mode_to_permission_mode);

    let Some(requested) = requested_mode else {
        return PermissionOverlayDecision {
            parent_mode,
            requested_mode,
            effective_mode: parent_mode,
            reason: PermissionOverlayReason::NoRequest,
        };
    };

    if parent_mode_locks_overlay(parent_mode) && requested != parent_mode {
        return PermissionOverlayDecision {
            parent_mode,
            requested_mode,
            effective_mode: parent_mode,
            reason: PermissionOverlayReason::ParentModeLocked,
        };
    }

    if requested == PermissionMode::BypassPermissions
        && parent_mode != PermissionMode::BypassPermissions
    {
        if !allow_bypass {
            return PermissionOverlayDecision {
                parent_mode,
                requested_mode,
                effective_mode: parent_mode,
                reason: PermissionOverlayReason::BlockedDangerousBypass,
            };
        }
        return PermissionOverlayDecision {
            parent_mode,
            requested_mode,
            effective_mode: requested,
            reason: PermissionOverlayReason::Applied,
        };
    }

    if is_permission_expansion(parent_mode, requested) {
        return PermissionOverlayDecision {
            parent_mode,
            requested_mode,
            effective_mode: parent_mode,
            reason: PermissionOverlayReason::BlockedExpansion,
        };
    }

    PermissionOverlayDecision {
        parent_mode,
        requested_mode,
        effective_mode: requested,
        reason: PermissionOverlayReason::Applied,
    }
}

pub fn resolve_subagent_agent_mode(
    parent_mode: &str,
    requested_mode: Option<&DefinitionPermissionMode>,
    allow_bypass: bool,
) -> AgentMode {
    resolve_permission_overlay(parent_mode, requested_mode, allow_bypass).agent_mode()
}

fn definition_mode_to_permission_mode(mode: &DefinitionPermissionMode) -> PermissionMode {
    match mode {
        DefinitionPermissionMode::Default => PermissionMode::Default,
        DefinitionPermissionMode::Plan => PermissionMode::Plan,
        DefinitionPermissionMode::Auto => PermissionMode::Auto,
        DefinitionPermissionMode::DontAsk => PermissionMode::DontAsk,
        DefinitionPermissionMode::BypassPermissions => PermissionMode::BypassPermissions,
        DefinitionPermissionMode::AcceptEdits => PermissionMode::AcceptEdits,
        DefinitionPermissionMode::Bubble => PermissionMode::Bubble,
    }
}

fn is_permission_expansion(parent: PermissionMode, requested: PermissionMode) -> bool {
    permission_rank(requested) > permission_rank(parent)
}

fn parent_mode_locks_overlay(mode: PermissionMode) -> bool {
    matches!(
        mode,
        PermissionMode::BypassPermissions | PermissionMode::AcceptEdits | PermissionMode::Auto
    )
}

fn permission_rank(mode: PermissionMode) -> u8 {
    match mode {
        PermissionMode::Plan => 0,
        PermissionMode::Default => 1,
        PermissionMode::Auto => 2,
        PermissionMode::AcceptEdits => 3,
        PermissionMode::Bubble => 4,
        PermissionMode::DontAsk => 5,
        PermissionMode::BypassPermissions => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(
        parent: &str,
        requested: DefinitionPermissionMode,
        allow_bypass: bool,
    ) -> PermissionOverlayDecision {
        resolve_permission_overlay(parent, Some(&requested), allow_bypass)
    }

    #[test]
    fn parent_plan_wins_over_agent_default() {
        let resolved = decision("plan", DefinitionPermissionMode::Default, false);

        assert_eq!(resolved.effective_mode, PermissionMode::Plan);
        assert_eq!(resolved.reason, PermissionOverlayReason::BlockedExpansion);
        assert_eq!(resolved.agent_mode(), AgentMode::Plan);
    }

    #[test]
    fn agent_plan_can_tighten_parent_default() {
        let resolved = decision("default", DefinitionPermissionMode::Plan, false);

        assert_eq!(resolved.effective_mode, PermissionMode::Plan);
        assert_eq!(resolved.reason, PermissionOverlayReason::Applied);
        assert_eq!(resolved.agent_mode(), AgentMode::Plan);
    }

    #[test]
    fn agent_cannot_widen_default_to_dont_ask() {
        let resolved = decision("default", DefinitionPermissionMode::DontAsk, false);

        assert_eq!(resolved.effective_mode, PermissionMode::Default);
        assert_eq!(resolved.reason, PermissionOverlayReason::BlockedExpansion);
        assert_eq!(resolved.agent_mode(), AgentMode::Normal);
    }

    #[test]
    fn agent_cannot_request_bypass_without_explicit_intent() {
        let resolved = decision(
            "default",
            DefinitionPermissionMode::BypassPermissions,
            false,
        );

        assert_eq!(resolved.effective_mode, PermissionMode::Default);
        assert_eq!(
            resolved.reason,
            PermissionOverlayReason::BlockedDangerousBypass
        );
        assert_eq!(resolved.agent_mode(), AgentMode::Normal);
    }

    #[test]
    fn agent_can_request_bypass_when_explicitly_allowed() {
        let resolved = decision("default", DefinitionPermissionMode::BypassPermissions, true);

        assert_eq!(resolved.effective_mode, PermissionMode::BypassPermissions);
        assert_eq!(resolved.reason, PermissionOverlayReason::Applied);
        assert_eq!(resolved.agent_mode(), AgentMode::Normal);
    }

    #[test]
    fn parent_bypass_mode_wins_over_agent_default() {
        let resolved = decision("bypassPermissions", DefinitionPermissionMode::Default, true);

        assert_eq!(resolved.effective_mode, PermissionMode::BypassPermissions);
        assert_eq!(resolved.reason, PermissionOverlayReason::ParentModeLocked);
        assert_eq!(resolved.agent_mode(), AgentMode::Normal);
    }

    #[test]
    fn parent_auto_mode_wins_over_agent_plan() {
        let resolved = decision("auto", DefinitionPermissionMode::Plan, false);

        assert_eq!(resolved.effective_mode, PermissionMode::Auto);
        assert_eq!(resolved.reason, PermissionOverlayReason::ParentModeLocked);
        assert_eq!(resolved.agent_mode(), AgentMode::Normal);
    }
}
