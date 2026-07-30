use super::*;
use archon_session::storage::{CompactionLedgerRecord, CompactionSegment, CompactionSummaryStatus};

fn message(role: &str, content: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"role": role, "content": content})
}

fn segment(start: u64, end: u64, status: CompactionSummaryStatus) -> CompactionSegment {
    CompactionSegment {
        id: format!("segment:s:{start}:{end}"),
        session_id: "s".into(),
        start_index: start,
        end_index: end,
        summary_status: status,
        summary: None,
        summary_model: None,
        summary_attribution: None,
        summary_failure: None,
        summary_input_tokens: None,
        summary_output_tokens: None,
        summary_cost: None,
        created_at: "2026-07-30T00:00:00Z".into(),
        updated_at: "2026-07-30T00:00:00Z".into(),
    }
}

#[test]
fn source_validation_rejects_missing_role_and_unmatched_tool_use() {
    assert!(validate_compaction_source(&[serde_json::Value::Null]).is_err());
    assert!(
        validate_compaction_source(&[serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "tool-1",
                "name": "Read",
                "input": {}
            }]
        })])
        .is_err()
    );
}

#[test]
fn source_validation_rejects_orphaned_tool_result() {
    let source = [serde_json::json!({
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": "fabricated",
            "content": "result"
        }]
    })];

    assert!(validate_compaction_source(&source).is_err());
}

#[test]
fn source_validation_rejects_malformed_or_unknown_content_blocks() {
    for source in [
        serde_json::json!({"role":"user","content":[{"type":"text"}]}),
        serde_json::json!({"role":"assistant","content":[{"type":"unknown"}]}),
    ] {
        assert!(validate_compaction_source(&[source]).is_err());
    }
}

#[test]
fn safe_segment_boundary_never_splits_tool_use_from_result() {
    let messages = vec![
        message("user", serde_json::json!("directive")),
        message(
            "assistant",
            serde_json::json!([{
                "type":"tool_use","id":"tool-1","name":"Read","input":{}
            }]),
        ),
        message(
            "user",
            serde_json::json!([{
                "type":"tool_result","tool_use_id":"tool-1","content":"ok"
            }]),
        ),
        message("user", serde_json::json!("recent")),
    ];

    let span = next_closed_segment_span(&messages, 0, 1).expect("safe segment");

    assert_eq!(span, SegmentSpan { start: 0, end: 2 });
}

#[test]
fn unsafe_tool_pair_crossing_preserve_boundary_closes_nothing() {
    let messages = vec![
        message("user", serde_json::json!("old")),
        message(
            "assistant",
            serde_json::json!([{
                "type":"tool_use","id":"tool-1","name":"Read","input":{}
            }]),
        ),
        message(
            "user",
            serde_json::json!([{
                "type":"tool_result","tool_use_id":"tool-1","content":"ok"
            }]),
        ),
    ];

    assert_eq!(next_closed_segment_span(&messages, 0, 1), None);
}

#[test]
fn redacted_segment_assembly_uses_non_sensitive_marker() {
    let source = vec![message("user", serde_json::json!("secret directive"))];
    let segments = vec![segment(0, 0, CompactionSummaryStatus::Redacted)];

    let assembled = assemble_compacted_messages(&source, &segments, &[], 0, 8_192, |_| {});
    let text = assembled.messages[0].to_string();

    assert!(!text.contains("secret directive"));
    assert!(!text.contains("Deterministic ledger"));
    assert!(text.contains("redacted"));
}

#[test]
fn deterministic_digest_carries_directive_tool_file_command_and_outcome() {
    let messages = vec![
        message("user", serde_json::json!("Never change the public API")),
        message(
            "assistant",
            serde_json::json!([{
                "type":"tool_use","id":"read-1","name":"Read",
                "input":{"file_path":"src/lib.rs"}
            },{
                "type":"tool_use","id":"bash-1","name":"Bash",
                "input":{"command":"cargo test"}
            }]),
        ),
        message(
            "user",
            serde_json::json!([{
                "type":"tool_result","tool_use_id":"bash-1",
                "content":"exit_code: 0\n100 tests passed","is_error":false
            }]),
        ),
    ];

    let digest = deterministic_segment_digest("segment:s:0:2", 0, &messages, 8_192);

    assert!(digest.contains("Never change the public API"));
    assert!(digest.contains("Read"));
    assert!(digest.contains("read-1"));
    assert!(digest.contains("src/lib.rs"));
    assert!(digest.contains("cargo test"));
    assert!(digest.contains("exit_code: 0"));
    assert!(digest.contains("100 tests passed"));
    assert!(
        serde_json::to_vec(&serde_json::Value::String(digest.clone()))
            .unwrap()
            .len()
            <= 8_192
    );
}

#[test]
fn digest_is_utf8_safe_and_bounded_at_provider_field_limit() {
    let messages = vec![message(
        "user",
        serde_json::json!(format!("DIRECTIVE {} END", "é".repeat(20_000))),
    )];

    let digest = deterministic_segment_digest("segment:s:0:0", 0, &messages, 1_024);

    assert!(std::str::from_utf8(digest.as_bytes()).is_ok());
    assert!(
        serde_json::to_vec(&serde_json::Value::String(digest.clone()))
            .unwrap()
            .len()
            <= 1_024
    );
    assert!(digest.contains("tool output trimmed"));
}

#[test]
fn ledger_derivation_is_deterministic_and_links_source_span() {
    let messages = vec![
        message("user", serde_json::json!("Keep the migration reversible")),
        message(
            "assistant",
            serde_json::json!([{
                "type":"tool_use","id":"bash-1","name":"Bash",
                "input":{"command":"cargo test"}
            }]),
        ),
        message(
            "user",
            serde_json::json!([{
                "type":"tool_result","tool_use_id":"bash-1",
                "content":"exit_code: 0","is_error":false
            }]),
        ),
    ];

    let first = derive_segment_ledger("s", 10, &messages, 8_192);
    let second = derive_segment_ledger("s", 10, &messages, 8_192);

    assert_eq!(first, second);
    assert!(first.iter().all(|record| {
        record.session_id == "s" && record.source_start_index >= 10 && record.source_end_index <= 12
    }));
    assert!(first.iter().any(|record| record.kind == "user_directive"));
    assert!(first.iter().any(|record| record.kind == "tool_call"));
    assert!(first.iter().any(|record| record.kind == "verified_outcome"));
}

#[test]
fn threshold_assembly_uses_stored_summary_and_never_mutates_source_messages() {
    let messages = vec![
        message("user", serde_json::json!("early directive")),
        message("assistant", serde_json::json!("early answer")),
        message("user", serde_json::json!("recent question")),
    ];
    let original = messages.clone();
    let mut stored = segment(0, 1, CompactionSummaryStatus::Succeeded);
    stored.summary = Some("stored summary".into());
    let ledger = vec![CompactionLedgerRecord {
        id: "ledger-1".into(),
        session_id: "s".into(),
        kind: "user_directive".into(),
        payload: "early directive".into(),
        source_start_index: 0,
        source_end_index: 0,
        created_at: "2026-07-30T00:00:00Z".into(),
    }];

    let assembled = assemble_compacted_messages(&messages, &[stored], &ledger, 1, 8_192, |_| {
        panic!("storage-only assembly must not invoke a provider")
    });

    assert_eq!(messages, original);
    assert!(assembled.messages[0].to_string().contains("stored summary"));
    assert!(
        assembled.messages[0]
            .to_string()
            .contains("early directive")
    );
    assert_eq!(assembled.messages.last().unwrap(), &messages[2]);
    assert_eq!(assembled.swapped_segment_ids, vec!["segment:s:0:1"]);
}

#[test]
fn failed_summary_assembles_deterministic_digest_without_deleting_source() {
    let messages = vec![
        message("user", serde_json::json!("early directive")),
        message("assistant", serde_json::json!("early answer")),
        message("user", serde_json::json!("recent question")),
    ];
    let original = messages.clone();
    let failed = segment(0, 1, CompactionSummaryStatus::Failed);

    let assembled = assemble_compacted_messages(&messages, &[failed], &[], 1, 2_048, |_| {
        panic!("digest fallback must not invoke a provider")
    });

    assert_eq!(messages, original);
    assert_eq!(assembled.digest_fallback_count, 1);
    assert!(
        assembled.messages[0]
            .to_string()
            .contains("early directive")
    );
    assert_eq!(assembled.messages.last().unwrap(), &messages[2]);
}

#[test]
fn recalled_segment_content_passes_provider_safe_field_cap() {
    let body = vec!["é".repeat(20_000)];

    let recalled = bound_recalled_segment("segment:s:0:0", &body, 1_024);

    assert!(
        serde_json::to_vec(&serde_json::Value::String(recalled.clone()))
            .unwrap()
            .len()
            <= 1_024
    );
    assert!(recalled.contains("segment:s:0:0"));
    assert!(recalled.contains("tool output trimmed"));
}

#[test]
fn explicit_compaction_model_wins_then_policy_default_then_active_fallback() {
    let available = ["active", "cheap"];

    assert_eq!(
        resolve_compaction_model(Some("cheap"), Some("policy"), "active", &available),
        ResolvedCompactionModel {
            model: "cheap".into(),
            source: CompactionModelSource::Explicit,
            fallback_reason: None,
        }
    );
    assert_eq!(
        resolve_compaction_model(None, Some("cheap"), "active", &available).source,
        CompactionModelSource::ProviderPolicy
    );
    let fallback = resolve_compaction_model(Some("missing"), Some("policy"), "active", &available);
    assert_eq!(fallback.model, "active");
    assert_eq!(fallback.source, CompactionModelSource::ActiveFallback);
    assert_eq!(
        fallback.fallback_reason.as_deref(),
        Some("explicit model unavailable; provider policy model unavailable")
    );
}
