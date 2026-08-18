use archon_permissions::mode::PermissionMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanEntryPath {
    SlashCommand,
    EnterPlanModeTool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanModeState {
    pub previous_permission_mode: Option<PermissionMode>,
    pub active_plan_id: Option<String>,
    pub entered_via: Option<PlanEntryPath>,
}

impl PlanModeState {
    pub fn record_entry(&mut self, previous_mode: PermissionMode, entry_path: PlanEntryPath) {
        if self.previous_permission_mode.is_none() {
            self.previous_permission_mode = Some(previous_mode);
            self.entered_via = Some(entry_path);
        }
    }
}

pub fn safe_restore_mode(recorded: Option<PermissionMode>, allow_bypass: bool) -> PermissionMode {
    match recorded {
        Some(PermissionMode::BypassPermissions) if !allow_bypass => PermissionMode::Default,
        Some(PermissionMode::Default) => PermissionMode::Default,
        Some(PermissionMode::AcceptEdits) => PermissionMode::AcceptEdits,
        Some(PermissionMode::Plan) => PermissionMode::Plan,
        Some(PermissionMode::Auto) => PermissionMode::Auto,
        Some(PermissionMode::DontAsk) => PermissionMode::DontAsk,
        Some(PermissionMode::Bubble) => PermissionMode::Bubble,
        Some(PermissionMode::BypassPermissions) => PermissionMode::BypassPermissions,
        None => PermissionMode::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_previous_mode_restores_default_not_auto() {
        assert_eq!(safe_restore_mode(None, false), PermissionMode::Default);
    }

    #[test]
    fn unavailable_bypass_mode_degrades_to_default() {
        assert_eq!(
            safe_restore_mode(Some(PermissionMode::BypassPermissions), false),
            PermissionMode::Default
        );
    }

    #[test]
    fn reentry_preserves_the_original_permission_mode() {
        let mut state = PlanModeState::default();
        state.record_entry(PermissionMode::Auto, PlanEntryPath::SlashCommand);
        state.record_entry(PermissionMode::Plan, PlanEntryPath::EnterPlanModeTool);
        assert_eq!(state.previous_permission_mode, Some(PermissionMode::Auto));
        assert_eq!(state.entered_via, Some(PlanEntryPath::SlashCommand));
    }

    #[test]
    fn every_allowed_mode_is_restored_unchanged() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
            PermissionMode::Auto,
            PermissionMode::DontAsk,
            PermissionMode::Bubble,
            PermissionMode::BypassPermissions,
        ] {
            assert_eq!(safe_restore_mode(Some(mode), true), mode);
        }
    }
}
