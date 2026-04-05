use archon_core::cli_flags::{FlagInput, resolve_flags};
use std::path::PathBuf;

/// Helper to create a default FlagInput with all None/false values.
fn empty_input() -> FlagInput {
    FlagInput {
        system_prompt: None,
        system_prompt_file: None,
        append_system_prompt: None,
        append_system_prompt_file: None,
        tools: None,
        allowed_tools: None,
        disallowed_tools: None,
        bare: false,
        disable_slash_commands: false,
        model: None,
        verbose: false,
        debug: None,
        debug_file: None,
        mcp_config: Vec::new(),
        strict_mcp_config: false,
        add_dir: Vec::new(),
        init: false,
        init_only: false,
        agent: None,
    }
}

#[test]
fn resolve_no_flags_returns_none() {
    let input = empty_input();
    let resolved = resolve_flags(&input).expect("should resolve");
    assert!(resolved.system_prompt_override.is_none());
    assert!(resolved.system_prompt_append.is_none());
    assert!(resolved.tool_whitelist.is_none());
    assert!(resolved.tool_blacklist.is_none());
    assert!(!resolved.bare_mode);
    assert!(!resolved.disable_slash_commands);
    assert!(resolved.model.is_none());
    assert!(!resolved.verbose);
    assert!(resolved.debug.is_none());
    assert!(resolved.debug_file.is_none());
    assert!(resolved.mcp_config_paths.is_empty());
    assert!(!resolved.strict_mcp_config);
    assert!(resolved.add_dirs.is_empty());
    assert!(!resolved.init);
    assert!(!resolved.init_only);
    assert!(resolved.agent.is_none());
    assert!(resolved.allowed_tools.is_none());
}

#[test]
fn resolve_system_prompt_text() {
    let mut input = empty_input();
    input.system_prompt = Some("custom system prompt".to_string());
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(
        resolved.system_prompt_override,
        Some("custom system prompt".to_string())
    );
}

#[test]
fn resolve_system_prompt_file() {
    // Create a temp file with known content
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("prompt.txt");
    std::fs::write(&file_path, "prompt from file").expect("write");

    let mut input = empty_input();
    input.system_prompt_file = Some(file_path);
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(
        resolved.system_prompt_override,
        Some("prompt from file".to_string())
    );
}

#[test]
fn resolve_system_prompt_file_missing() {
    let mut input = empty_input();
    input.system_prompt_file = Some(PathBuf::from("/nonexistent/path/to/prompt.txt"));
    let result = resolve_flags(&input);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("system-prompt-file"),
        "error should mention system-prompt-file, got: {err}"
    );
}

#[test]
fn resolve_append_system_prompt() {
    let mut input = empty_input();
    input.append_system_prompt = Some("extra instructions".to_string());
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(
        resolved.system_prompt_append,
        Some("extra instructions".to_string())
    );
}

#[test]
fn resolve_append_system_prompt_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file_path = dir.path().join("append.txt");
    std::fs::write(&file_path, "appended from file").expect("write");

    let mut input = empty_input();
    input.append_system_prompt_file = Some(file_path);
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(
        resolved.system_prompt_append,
        Some("appended from file".to_string())
    );
}

#[test]
fn resolve_append_system_prompt_file_missing() {
    let mut input = empty_input();
    input.append_system_prompt_file = Some(PathBuf::from("/nonexistent/append.txt"));
    let result = resolve_flags(&input);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("append-system-prompt-file"),
        "error should mention append-system-prompt-file, got: {err}"
    );
}

#[test]
fn resolve_bare_mode() {
    let mut input = empty_input();
    input.bare = true;
    let resolved = resolve_flags(&input).expect("should resolve");
    assert!(resolved.bare_mode);
}

#[test]
fn resolve_tool_whitelist() {
    let mut input = empty_input();
    input.tools = Some(vec!["Read".to_string(), "Write".to_string()]);
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(
        resolved.tool_whitelist,
        Some(vec!["Read".to_string(), "Write".to_string()])
    );
}

#[test]
fn resolve_tool_blacklist() {
    let mut input = empty_input();
    input.disallowed_tools = Some(vec!["Bash".to_string()]);
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(resolved.tool_blacklist, Some(vec!["Bash".to_string()]));
}

#[test]
fn resolve_allowed_tools() {
    let mut input = empty_input();
    input.allowed_tools = Some(vec!["Read".to_string(), "Grep".to_string()]);
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(
        resolved.allowed_tools,
        Some(vec!["Read".to_string(), "Grep".to_string()])
    );
}

#[test]
fn resolve_model_flag() {
    let mut input = empty_input();
    input.model = Some("claude-opus-4-20250514".to_string());
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(resolved.model, Some("claude-opus-4-20250514".to_string()));
}

#[test]
fn resolve_verbose() {
    let mut input = empty_input();
    input.verbose = true;
    let resolved = resolve_flags(&input).expect("should resolve");
    assert!(resolved.verbose);
}

#[test]
fn resolve_debug_enabled_no_filter() {
    let mut input = empty_input();
    input.debug = Some(None);
    let resolved = resolve_flags(&input).expect("should resolve");
    // debug is Some(None) meaning enabled with no category filter
    assert!(resolved.debug.is_some());
    assert!(resolved.debug.unwrap().is_none());
}

#[test]
fn resolve_debug_enabled_with_filter() {
    let mut input = empty_input();
    input.debug = Some(Some("mcp,agent".to_string()));
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(resolved.debug, Some(Some("mcp,agent".to_string())));
}

#[test]
fn resolve_debug_file() {
    let mut input = empty_input();
    input.debug_file = Some(PathBuf::from("/tmp/debug.log"));
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(resolved.debug_file, Some(PathBuf::from("/tmp/debug.log")));
}

#[test]
fn resolve_disable_slash_commands() {
    let mut input = empty_input();
    input.disable_slash_commands = true;
    let resolved = resolve_flags(&input).expect("should resolve");
    assert!(resolved.disable_slash_commands);
}

#[test]
fn resolve_mcp_config_paths() {
    let mut input = empty_input();
    input.mcp_config = vec![
        PathBuf::from("/path/to/mcp1.json"),
        PathBuf::from("/path/to/mcp2.json"),
    ];
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(resolved.mcp_config_paths.len(), 2);
}

#[test]
fn resolve_strict_mcp_config() {
    let mut input = empty_input();
    input.strict_mcp_config = true;
    let resolved = resolve_flags(&input).expect("should resolve");
    assert!(resolved.strict_mcp_config);
}

#[test]
fn resolve_add_dirs() {
    let mut input = empty_input();
    input.add_dir = vec![PathBuf::from("/tmp"), PathBuf::from("/home")];
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(resolved.add_dirs.len(), 2);
}

#[test]
fn resolve_init_flags() {
    let mut input = empty_input();
    input.init = true;
    let resolved = resolve_flags(&input).expect("should resolve");
    assert!(resolved.init);

    let mut input2 = empty_input();
    input2.init_only = true;
    let resolved2 = resolve_flags(&input2).expect("should resolve");
    assert!(resolved2.init_only);
}

#[test]
fn resolve_agent() {
    let mut input = empty_input();
    input.agent = Some("backend-dev".to_string());
    let resolved = resolve_flags(&input).expect("should resolve");
    assert_eq!(resolved.agent, Some("backend-dev".to_string()));
}

// Test tool registry filtering
use archon_core::dispatch::create_default_registry;

#[test]
fn filter_whitelist_retains_only_named_tools() {
    let mut registry = create_default_registry(std::env::temp_dir());
    registry.filter_whitelist(&["Read", "Write"]);
    let names = registry.tool_names();
    assert!(names.contains(&"Read"));
    assert!(names.contains(&"Write"));
    assert!(!names.contains(&"Bash"));
    assert!(!names.contains(&"Grep"));
    assert_eq!(names.len(), 2);
}

#[test]
fn filter_blacklist_removes_named_tools() {
    let mut registry = create_default_registry(std::env::temp_dir());
    let original_count = registry.tool_names().len();
    registry.filter_blacklist(&["Bash", "PowerShell"]);
    let names = registry.tool_names();
    assert!(!names.contains(&"Bash"));
    assert!(!names.contains(&"PowerShell"));
    assert!(names.contains(&"Read"));
    assert_eq!(names.len(), original_count - 2);
}

#[test]
fn filter_whitelist_empty_list_removes_all() {
    let mut registry = create_default_registry(std::env::temp_dir());
    registry.filter_whitelist(&[]);
    assert!(registry.tool_names().is_empty());
}

#[test]
fn filter_blacklist_unknown_tool_is_noop() {
    let mut registry = create_default_registry(std::env::temp_dir());
    let original_count = registry.tool_names().len();
    registry.filter_blacklist(&["NonexistentTool"]);
    assert_eq!(registry.tool_names().len(), original_count);
}
