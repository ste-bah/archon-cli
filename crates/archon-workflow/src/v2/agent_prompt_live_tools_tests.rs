//! Obs-119: a call whose declared tools include an external application's
//! check tool is told to load the task's source first and never to save.
use super::*;
use serde_json::json;

#[test]
fn check_tools_get_the_load_first_and_no_save_rule_naming_the_declared_loader() {
    let input = json!({"item": {"required_tools": [
        "mcp__editor__script_compile", "mcp__editor__get_errors",
        "mcp__editor__set_source", "mcp__editor__quote_get", "cargo"
    ]}});
    let section = live_state_tools_prompt_section(&input);
    assert!(section.starts_with("## Live-State Tools\n"), "{section}");
    assert!(
        section.contains(
            "check content in an external application: `mcp__editor__script_compile`, \
             `mcp__editor__get_errors`."
        ),
        "{section}"
    );
    assert!(
        section.contains("with the declared loader `mcp__editor__set_source`."),
        "{section}"
    );
    assert!(section.contains("never count its errors or its success as this task's"));
    assert!(section.contains("Never call a tool that saves, publishes"));
    assert!(!section.contains("quote_get") && !section.contains("`cargo`"));
}

#[test]
fn without_a_declared_loader_the_call_is_told_it_cannot_load_and_a_declared_saver_is_named() {
    let input =
        json!({"required_tools": ["mcp__editor__smart_compile", "mcp__editor__save_script"]});
    let section = live_state_tools_prompt_section(&input);
    assert!(
        section.contains("this task declares none, so you cannot load it"),
        "{section}"
    );
    assert!(
        section.contains(
            "This task declares `mcp__editor__save_script`: call it only as the task says."
        ),
        "{section}"
    );
}

#[test]
fn no_mcp_check_tool_renders_nothing() {
    for input in [
        json!({"required_tools": ["mcp__data__quote_get", "cargo", "check"]}),
        json!({"required_tools": []}),
        json!({}),
    ] {
        assert_eq!(live_state_tools_prompt_section(&input), "", "{input}");
    }
}
