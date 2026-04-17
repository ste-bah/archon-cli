use archon_tui::events::TuiEvent;

#[test]
fn text_delta_variant() {
    let _e = TuiEvent::TextDelta("hello".into());
}

#[test]
fn thinking_delta_variant() {
    let _e = TuiEvent::ThinkingDelta("thinking".into());
}

#[test]
fn tool_start_variant() {
    let _e = TuiEvent::ToolStart {
        name: "Read".into(),
        id: "tool-1".into(),
    };
}

#[test]
fn tool_complete_variant() {
    let _e = TuiEvent::ToolComplete {
        name: "Read".into(),
        id: "tool-1".into(),
        success: true,
        output: "file contents".into(),
    };
}

#[test]
fn turn_complete_variant() {
    let _e = TuiEvent::TurnComplete {
        input_tokens: 100,
        output_tokens: 200,
    };
}

#[test]
fn error_variant() {
    let _e = TuiEvent::Error("something went wrong".into());
}

#[test]
fn generation_started_variant() {
    let _e = TuiEvent::GenerationStarted;
}

#[test]
fn slash_command_complete_variant() {
    let _e = TuiEvent::SlashCommandComplete;
}

#[test]
fn thinking_toggle_variant() {
    let _e = TuiEvent::ThinkingToggle(true);
    let _e = TuiEvent::ThinkingToggle(false);
}

#[test]
fn model_changed_variant() {
    let _e = TuiEvent::ModelChanged("claude-sonnet-4-6".into());
}

#[test]
fn btw_response_variant() {
    let _e = TuiEvent::BtwResponse("btw response text".into());
}

#[test]
fn permission_prompt_variant() {
    let _e = TuiEvent::PermissionPrompt {
        tool: "Bash".into(),
        description: "Run shell command".into(),
    };
}

#[test]
fn session_renamed_variant() {
    let _e = TuiEvent::SessionRenamed("my-session".into());
}

#[test]
fn permission_mode_changed_variant() {
    let _e = TuiEvent::PermissionModeChanged("auto".into());
}

#[test]
fn show_session_picker_variant() {
    use archon_tui::app::SessionPickerEntry;
    let entries = vec![SessionPickerEntry {
        id: "sess-1".into(),
        name: "test session".into(),
        turns: 5,
        cost: 0.05,
        last_active: "2m ago".into(),
    }];
    let _e = TuiEvent::ShowSessionPicker(entries);
}

#[test]
fn set_accent_color_variant() {
    use ratatui::style::Color;
    let _e = TuiEvent::SetAccentColor(Color::Cyan);
}

#[test]
fn set_theme_variant() {
    let _e = TuiEvent::SetTheme("gruvbox".into());
}

#[test]
fn show_mcp_manager_variant() {
    use archon_tui::app::McpServerEntry;
    let servers = vec![McpServerEntry {
        name: "filesystem".into(),
        state: "ready".into(),
        tool_count: 3,
        disabled: false,
        tools: vec!["mcp__fs__read".into()],
    }];
    let _e = TuiEvent::ShowMcpManager(servers);
}

#[test]
fn update_mcp_manager_variant() {
    use archon_tui::app::McpServerEntry;
    let servers = vec![McpServerEntry {
        name: "filesystem".into(),
        state: "ready".into(),
        tool_count: 3,
        disabled: false,
        tools: vec!["mcp__fs__read".into()],
    }];
    let _e = TuiEvent::UpdateMcpManager(servers);
}

#[test]
fn set_vim_mode_variant() {
    let _e = TuiEvent::SetVimMode(true);
    let _e = TuiEvent::SetVimMode(false);
}

#[test]
fn vim_toggle_variant() {
    let _e = TuiEvent::VimToggle;
}

#[test]
fn voice_text_variant() {
    let _e = TuiEvent::VoiceText("transcribed voice".into());
}

#[test]
fn set_agent_info_variant() {
    let _e = TuiEvent::SetAgentInfo {
        name: "coder".into(),
        color: Some("#00ff00".into()),
    };
    let _e = TuiEvent::SetAgentInfo {
        name: "reviewer".into(),
        color: None,
    };
}

#[test]
fn resize_variant() {
    let _e = TuiEvent::Resize { cols: 80, rows: 24 };
}

#[test]
fn user_input_variant() {
    let _e = TuiEvent::UserInput("hello world".into());
}

#[test]
fn slash_cancel_variant() {
    let _e = TuiEvent::SlashCancel;
}

#[test]
fn slash_agent_variant() {
    let _e = TuiEvent::SlashAgent("agent-123".into());
}

#[test]
fn done_variant() {
    let _e = TuiEvent::Done;
}

#[test]
fn notification_timeout_variant() {
    let _e = TuiEvent::NotificationTimeout(5000);
}

// -----------------------------------------------------------------
// TASK-AGS-822: OpenView(ViewId) round-trip + per-variant ViewId
// coverage. `ViewId` is defined at layer 0 (events.rs) and re-exported
// from `crate::app::ViewId` per the SessionPickerEntry / McpServerEntry
// precedent. These tests exercise BOTH access paths to pin the
// re-export contract (body-migrates AGS-806..819 will consume
// `archon_tui::app::ViewId`).
// -----------------------------------------------------------------

#[test]
fn open_view_variant() {
    // Constructs OpenView via the re-export path. `archon_tui::app::ViewId`
    // must resolve to the same type defined in `archon_tui::events::ViewId`.
    use archon_tui::app::ViewId;
    let _e = TuiEvent::OpenView(ViewId::Tasks);
}

#[test]
fn view_id_tasks() {
    use archon_tui::app::ViewId;
    let v = ViewId::Tasks;
    let cloned = v;
    assert_eq!(v, cloned);
}

#[test]
fn view_id_settings() {
    use archon_tui::app::ViewId;
    let v = ViewId::Settings;
    let cloned = v;
    assert_eq!(v, cloned);
}

#[test]
fn view_id_context() {
    use archon_tui::app::ViewId;
    let v = ViewId::Context;
    let cloned = v;
    assert_eq!(v, cloned);
}

#[test]
fn view_id_memory_browser() {
    use archon_tui::app::ViewId;
    let v = ViewId::MemoryBrowser;
    let cloned = v;
    assert_eq!(v, cloned);
}

#[test]
fn view_id_model_picker() {
    use archon_tui::app::ViewId;
    let v = ViewId::ModelPicker;
    let cloned = v;
    assert_eq!(v, cloned);
}

#[test]
fn view_id_status() {
    use archon_tui::app::ViewId;
    let v = ViewId::Status;
    let cloned = v;
    assert_eq!(v, cloned);
}
