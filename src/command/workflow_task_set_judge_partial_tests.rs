use super::*;

#[test]
fn ordinary_words_do_not_expand_redacted_bytes_before_credit() {
    let temp = tempfile::tempdir().unwrap();
    let progress = FreezeProgress::default();
    let partial = PartialReply::new(temp.path(), "batch", &progress);
    let text = format!("{{\"reason\":\"{}", "token ".repeat(700_000));
    assert!(text.len() < MAX_PARTIAL_REPLY_BYTES);
    assert_eq!(archon_observability::redaction::redact_text(&text), text);
    assert!(partial.extended(&text, 1, text.len()));
    assert_eq!(
        progress.total(),
        1,
        "the bounded reply earns its chunk credit"
    );
    assert_eq!(
        partial.open_reply().as_ref().map(|(reply, _)| reply),
        Some(&text)
    );
}

#[test]
fn shrinking_redaction_keeps_withdrawable_credit() {
    let temp = tempfile::tempdir().unwrap();
    let progress = FreezeProgress::default();
    let partial = PartialReply::new(temp.path(), "batch", &progress);
    let mut text = String::from("{\"reason\":\"sk-");
    partial.extended(&text, 1, text.len());
    for chunks in 2..=1001 {
        text.push('a');
        partial.extended(&text, chunks, 1);
    }
    assert_eq!(progress.total(), 1001);
    assert!(
        partial.open_reply().is_some(),
        "redaction never invalidates the counter"
    );
    partial.spent();
    assert_eq!(progress.total(), 0, "all spent chunks are withdrawn");
}

/// Round 5: a cache whose byte credit is past the reply cap, or whose chunks
/// exceed its credit, is rejected: it never inflates the host's progress.
#[test]
fn an_unbounded_credit_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let progress = FreezeProgress::default();
    let partial = PartialReply::new(temp.path(), "batch", &progress);
    let text = "{\"reason\":\"abc".to_string();
    partial.extended(&text, 1, text.len());
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&partial.path).unwrap()).unwrap();
    saved["credit_bytes"] = serde_json::json!(u64::MAX);
    saved["chunks"] = serde_json::json!(1u64 << 40);
    std::fs::write(&partial.path, serde_json::to_vec(&saved).unwrap()).unwrap();
    let fresh = FreezeProgress::default();
    PartialReply::new(temp.path(), "batch", &fresh).count_saved(false);
    assert_eq!(fresh.total(), 0, "a corrupt credit counts nothing");
}
