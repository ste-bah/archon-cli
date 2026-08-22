//! `TuiEvent::variant_name`, split out for the 500-line file-size gate.
//!
//! One arm per variant and nothing else, which is why it is the half of
//! `events.rs` worth moving: it grows with every event added and says nothing
//! about what any of them mean.

use super::events::TuiEvent;

impl TuiEvent {
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::TextDelta(_) => "TextDelta",
            Self::ThinkingDelta(_) => "ThinkingDelta",
            Self::TransientThinkingDelta(_) => "TransientThinkingDelta",
            Self::CommitThinkingPreview => "CommitThinkingPreview",
            Self::DiscardThinkingPreview => "DiscardThinkingPreview",
            Self::ToolStart { .. } => "ToolStart",
            Self::ToolOutputChunk { .. } => "ToolOutputChunk",
            Self::ToolComplete { .. } => "ToolComplete",
            Self::TurnComplete { .. } => "TurnComplete",
            Self::Error(_) => "Error",
            Self::GenerationStarted => "GenerationStarted",
            Self::SlashCommandComplete => "SlashCommandComplete",
            Self::ThinkingToggle(_) => "ThinkingToggle",
            Self::OpenThinkingArchive => "OpenThinkingArchive",
            Self::ModelChanged(_) => "ModelChanged",
            Self::BtwResponse(_) => "BtwResponse",
            Self::PermissionPrompt { .. } => "PermissionPrompt",
            Self::AskUserPrompt { .. } => "AskUserPrompt",
            Self::SessionRenamed(_) => "SessionRenamed",
            Self::PermissionModeChanged(_) => "PermissionModeChanged",
            Self::ShowSessionPicker(_) => "ShowSessionPicker",
            Self::SetAccentColor(_) => "SetAccentColor",
            Self::SetTheme(_) => "SetTheme",
            Self::ShowMcpManager(_) => "ShowMcpManager",
            Self::UpdateMcpManager(_) => "UpdateMcpManager",
            Self::ShowMessageSelector(_) => "ShowMessageSelector",
            Self::ShowSkillsMenu(_) => "ShowSkillsMenu",
            Self::ShowModelPicker(_) => "ShowModelPicker",
            Self::ShowThemePicker(_) => "ShowThemePicker",
            Self::ShowSettings(_) => "ShowSettings",
            Self::ShowHooks(_) => "ShowHooks",
            Self::ShowPermissions { .. } => "ShowPermissions",
            Self::ShowPermissionPresets { .. } => "ShowPermissionPresets",
            Self::ShowMemoryFiles(_) => "ShowMemoryFiles",
            Self::ShowBranchPicker(_) => "ShowBranchPicker",
            Self::ShowVoiceCapture { .. } => "ShowVoiceCapture",
            Self::ShowTokenAttribution(_) => "ShowTokenAttribution",
            Self::VoiceRecording(_) => "VoiceRecording",
            Self::VoiceLevel(_) => "VoiceLevel",
            Self::ShowFilePicker { .. } => "ShowFilePicker",
            Self::ShowSearchResults { .. } => "ShowSearchResults",
            Self::OpenView(_) => "OpenView",
            Self::OpenViewRows { .. } => "OpenViewRows",
            Self::VideoIngestProgress(_) => "VideoIngestProgress",
            Self::AgentActivity(_) => "AgentActivity",
            Self::ActivityStream(_) => "ActivityStream",
            Self::ContextPressureUpdated { .. } => "ContextPressureUpdated",
            Self::SetVimMode(_) => "SetVimMode",
            Self::VimToggle => "VimToggle",
            Self::VoiceText(_) => "VoiceText",
            Self::SetAgentInfo { .. } => "SetAgentInfo",
            Self::Resize { .. } => "Resize",
            Self::Done => "Done",
            Self::NotificationTimeout(_) => "NotificationTimeout",
        }
    }
}
