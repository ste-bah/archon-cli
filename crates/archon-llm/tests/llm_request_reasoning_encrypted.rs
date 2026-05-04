use archon_llm::provider::LlmRequest;

fn blob_4kb() -> String {
    "r".repeat(4096)
}

#[test]
fn default_is_none() {
    assert_eq!(LlmRequest::default().reasoning_encrypted, None);
}

#[test]
fn builder_sets_some() {
    let request = LlmRequest::default().with_reasoning_encrypted(Some("blob".into()));

    assert_eq!(request.reasoning_encrypted, Some("blob".into()));
}

#[test]
fn builder_sets_none_clears() {
    let request = LlmRequest::default()
        .with_reasoning_encrypted(Some("blob".into()))
        .with_reasoning_encrypted(None);

    assert_eq!(request.reasoning_encrypted, None);
}

#[test]
fn field_is_4kb_safe() {
    let blob = blob_4kb();
    let request = LlmRequest::default().with_reasoning_encrypted(Some(blob.clone()));

    assert_eq!(request.reasoning_encrypted.as_deref(), Some(blob.as_str()));
    assert_eq!(request.reasoning_encrypted.as_deref().map(str::len), Some(4096));
}
