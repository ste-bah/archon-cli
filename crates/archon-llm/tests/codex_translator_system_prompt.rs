use archon_llm::providers::codex::translator::join_system_prompt;

#[test]
fn empty_system_prompt_returns_none() {
    assert_eq!(join_system_prompt(&[]), None);
}

#[test]
fn joins_canonical_system_text_blocks() {
    let system = vec![
        serde_json::json!({"type": "text", "text": "one"}),
        serde_json::json!({"type": "text", "text": "two", "cache_control": {"type": "ephemeral"}}),
    ];

    assert_eq!(join_system_prompt(&system).as_deref(), Some("one\n\ntwo"));
}

#[test]
fn ignores_openai_role_content_system_blocks() {
    let system = vec![serde_json::json!({"role": "system", "content": "wrong"})];

    assert_eq!(join_system_prompt(&system), None);
}
