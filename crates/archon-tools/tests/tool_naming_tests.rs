use archon_tools::validation::{
    KNOWN_MODEL_IDS, KNOWN_SHORTCUTS, LEGACY_PERMISSION_ALIASES, VALID_EFFORT_LEVELS,
    VALID_PERMISSION_MODES, edit_distance, validate_effort_level, validate_model_name,
    validate_permission_mode,
};

// ---------------------------------------------------------------------------
// Tool name verification
// ---------------------------------------------------------------------------

#[test]
fn builtin_tool_names_match_claude_code_conventions() {
    use archon_tools::tool::Tool;

    let expected: Vec<&str> = vec![
        "Bash",
        "Read",
        "Write",
        "Edit",
        "Glob",
        "Grep",
        "WebFetch",
        "AskUserQuestion",
        "Agent",
        "SendMessage",
        "TodoWrite",
        "Sleep",
        "EnterPlanMode",
        "ExitPlanMode",
        "ToolSearch",
        "Config",
        "PowerShell",
    ];

    // Instantiate all concrete tool structs and check their name() methods.
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(archon_tools::bash::BashTool::default()),
        Box::new(archon_tools::file_read::ReadTool),
        Box::new(archon_tools::file_write::WriteTool),
        Box::new(archon_tools::file_edit::EditTool),
        Box::new(archon_tools::glob_tool::GlobTool),
        Box::new(archon_tools::grep::GrepTool),
        Box::new(archon_tools::webfetch::WebFetchTool),
        Box::new(archon_tools::ask_user::AskUserTool),
        Box::new(archon_tools::agent_tool::AgentTool),
        Box::new(archon_tools::send_message::SendMessageTool),
        Box::new(archon_tools::todo_write::TodoWriteTool),
        Box::new(archon_tools::sleep::SleepTool),
        Box::new(archon_tools::plan_mode::EnterPlanModeTool),
        Box::new(archon_tools::plan_mode::ExitPlanModeTool),
        Box::new(archon_tools::toolsearch::ToolSearchTool::new(vec![])),
        Box::new(archon_tools::config_tool::ConfigTool),
        Box::new(archon_tools::powershell::PowerShellTool::default()),
    ];

    let mut actual_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    actual_names.sort();

    let mut expected_sorted = expected.clone();
    expected_sorted.sort();

    assert_eq!(
        actual_names, expected_sorted,
        "Tool names do not match expected Claude Code conventions"
    );
}

// ---------------------------------------------------------------------------
// Constants integrity
// ---------------------------------------------------------------------------

#[test]
fn known_shortcuts_are_complete() {
    assert_eq!(KNOWN_SHORTCUTS.len(), 3);
    assert!(KNOWN_SHORTCUTS.iter().any(|(s, _)| *s == "opus"));
    assert!(KNOWN_SHORTCUTS.iter().any(|(s, _)| *s == "sonnet"));
    assert!(KNOWN_SHORTCUTS.iter().any(|(s, _)| *s == "haiku"));
}

#[test]
fn known_model_ids_are_complete() {
    assert_eq!(KNOWN_MODEL_IDS.len(), 3);
    assert!(KNOWN_MODEL_IDS.contains(&"claude-opus-4-6"));
    assert!(KNOWN_MODEL_IDS.contains(&"claude-sonnet-4-6"));
    assert!(KNOWN_MODEL_IDS.contains(&"claude-haiku-4-5-20251001"));
}

#[test]
fn valid_effort_levels_are_complete() {
    assert_eq!(VALID_EFFORT_LEVELS, &["high", "medium", "low"]);
}

#[test]
fn valid_permission_modes_are_complete() {
    assert_eq!(
        VALID_PERMISSION_MODES,
        &[
            "default",
            "acceptEdits",
            "plan",
            "auto",
            "dontAsk",
            "bypassPermissions"
        ]
    );
}

#[test]
fn legacy_permission_aliases_are_complete() {
    assert_eq!(LEGACY_PERMISSION_ALIASES.len(), 2);
    assert!(LEGACY_PERMISSION_ALIASES.contains(&("ask", "default")));
    assert!(LEGACY_PERMISSION_ALIASES.contains(&("yolo", "bypassPermissions")));
}

// ---------------------------------------------------------------------------
// validate_model_name
// ---------------------------------------------------------------------------

#[test]
fn model_shortcut_opus() {
    assert_eq!(validate_model_name("opus"), Ok("claude-opus-4-6".into()));
}

#[test]
fn model_shortcut_sonnet() {
    assert_eq!(
        validate_model_name("sonnet"),
        Ok("claude-sonnet-4-6".into())
    );
}

#[test]
fn model_shortcut_haiku() {
    assert_eq!(
        validate_model_name("haiku"),
        Ok("claude-haiku-4-5-20251001".into())
    );
}

#[test]
fn model_full_id_opus() {
    assert_eq!(
        validate_model_name("claude-opus-4-6"),
        Ok("claude-opus-4-6".into())
    );
}

#[test]
fn model_full_id_sonnet() {
    assert_eq!(
        validate_model_name("claude-sonnet-4-6"),
        Ok("claude-sonnet-4-6".into())
    );
}

#[test]
fn model_shortcut_case_insensitive() {
    assert_eq!(validate_model_name("OPUS"), Ok("claude-opus-4-6".into()));
    assert_eq!(
        validate_model_name("Sonnet"),
        Ok("claude-sonnet-4-6".into())
    );
}

#[test]
fn model_fuzzy_opsi_suggests_opus() {
    let result = validate_model_name("opsi");
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("Did you mean 'opus'?"),
        "Expected suggestion for 'opus', got: {msg}"
    );
}

#[test]
fn model_too_far_haikiruus_no_suggestion() {
    let result = validate_model_name("haikiruus");
    assert!(result.is_err());
    let msg = result.unwrap_err();
    // Distance to "haiku" is 4 (> 2), so no suggestion
    assert!(
        !msg.contains("Did you mean"),
        "Should NOT suggest anything for distance > 2, got: {msg}"
    );
    // Should list valid options
    assert!(msg.contains("opus"), "Should list valid shortcuts: {msg}");
    assert!(msg.contains("sonnet"), "Should list valid shortcuts: {msg}");
    assert!(msg.contains("haiku"), "Should list valid shortcuts: {msg}");
}

#[test]
fn model_totally_unknown_lists_valid_options() {
    let result = validate_model_name("xyz123");
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("opus"));
    assert!(msg.contains("sonnet"));
    assert!(msg.contains("haiku"));
}

// ---------------------------------------------------------------------------
// validate_effort_level
// ---------------------------------------------------------------------------

#[test]
fn effort_high() {
    assert_eq!(validate_effort_level("high"), Ok("high".into()));
}

#[test]
fn effort_medium() {
    assert_eq!(validate_effort_level("medium"), Ok("medium".into()));
}

#[test]
fn effort_low() {
    assert_eq!(validate_effort_level("low"), Ok("low".into()));
}

#[test]
fn effort_case_insensitive() {
    assert_eq!(validate_effort_level("HIGH"), Ok("high".into()));
    assert_eq!(validate_effort_level("Medium"), Ok("medium".into()));
    assert_eq!(validate_effort_level("LOW"), Ok("low".into()));
}

#[test]
fn effort_invalid_critical() {
    let result = validate_effort_level("critical");
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("high") && msg.contains("medium") && msg.contains("low"),
        "Should list valid levels: {msg}"
    );
}

#[test]
fn effort_fuzzy_hig_suggests_high() {
    let result = validate_effort_level("hig");
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("Did you mean 'high'?"),
        "Expected suggestion for 'high', got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// validate_permission_mode
// ---------------------------------------------------------------------------

#[test]
fn permission_default() {
    assert_eq!(validate_permission_mode("default"), Ok("default".into()));
}

#[test]
fn permission_plan() {
    assert_eq!(validate_permission_mode("plan"), Ok("plan".into()));
}

#[test]
fn permission_auto() {
    assert_eq!(validate_permission_mode("auto"), Ok("auto".into()));
}

#[test]
fn permission_accept_edits() {
    assert_eq!(
        validate_permission_mode("acceptEdits"),
        Ok("acceptEdits".into())
    );
}

#[test]
fn permission_dont_ask() {
    assert_eq!(validate_permission_mode("dontAsk"), Ok("dontAsk".into()));
}

#[test]
fn permission_bypass_permissions() {
    assert_eq!(
        validate_permission_mode("bypassPermissions"),
        Ok("bypassPermissions".into())
    );
}

#[test]
fn permission_legacy_ask() {
    assert_eq!(validate_permission_mode("ask"), Ok("default".into()));
}

#[test]
fn permission_legacy_yolo() {
    assert_eq!(
        validate_permission_mode("yolo"),
        Ok("bypassPermissions".into())
    );
}

#[test]
fn permission_fuzzy_yoloo_suggests_yolo() {
    let result = validate_permission_mode("yoloo");
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("Did you mean 'yolo'?"),
        "Expected suggestion for 'yolo', got: {msg}"
    );
}

#[test]
fn permission_fuzzy_plam_suggests_plan() {
    let result = validate_permission_mode("plam");
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("Did you mean 'plan'?"),
        "Expected suggestion for 'plan', got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Error format parity — all tool errors must start with "Error: "
// ---------------------------------------------------------------------------

#[test]
fn tool_result_error_has_error_prefix() {
    use archon_tools::tool::ToolResult;

    // Errors without prefix get it added
    let r = ToolResult::error("file not found");
    assert!(
        r.content.starts_with("Error: "),
        "ToolResult::error should prepend 'Error: ' prefix, got: {}",
        r.content
    );
    assert_eq!(r.content, "Error: file not found");
}

#[test]
fn tool_result_error_no_double_prefix() {
    use archon_tools::tool::ToolResult;

    // Errors already prefixed should not get doubled
    let r = ToolResult::error("Error: already prefixed");
    assert_eq!(
        r.content, "Error: already prefixed",
        "Should not double-prefix, got: {}",
        r.content
    );
}

#[test]
fn tool_result_error_preserves_details() {
    use archon_tools::tool::ToolResult;

    let r = ToolResult::error("file_path is required and must be a string");
    assert_eq!(
        r.content,
        "Error: file_path is required and must be a string"
    );
}

#[test]
fn tool_result_error_with_multiline_details() {
    use archon_tools::tool::ToolResult;

    let msg = "Failed to read file: /tmp/foo.txt\n\nPermission denied (os error 13)";
    let r = ToolResult::error(msg);
    assert!(r.content.starts_with("Error: Failed to read file:"));
    assert!(r.content.contains("Permission denied"));
}

// ---------------------------------------------------------------------------
// edit_distance
// ---------------------------------------------------------------------------

#[test]
fn distance_identical() {
    assert_eq!(edit_distance("opus", "opus"), 0);
}

#[test]
fn distance_one_substitution() {
    assert_eq!(edit_distance("opus", "opsi"), 2);
}

#[test]
fn distance_one_substitution_opux() {
    assert_eq!(edit_distance("opus", "opux"), 1);
}

#[test]
fn distance_large() {
    assert_eq!(edit_distance("haiku", "haikiruus"), 4);
}

#[test]
fn distance_empty_to_string() {
    assert_eq!(edit_distance("", "abc"), 3);
}

#[test]
fn distance_string_to_empty() {
    assert_eq!(edit_distance("abc", ""), 3);
}

#[test]
fn distance_both_empty() {
    assert_eq!(edit_distance("", ""), 0);
}

#[test]
fn distance_case_insensitive() {
    assert_eq!(edit_distance("Opus", "opus"), 0);
    assert_eq!(edit_distance("HIGH", "high"), 0);
}
