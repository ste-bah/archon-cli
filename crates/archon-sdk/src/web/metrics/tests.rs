use std::path::Path;

use archon_learning::runtime_models::ProviderRuntimeEventRecord;

use super::*;

#[test]
fn missing_event_ledger_is_empty() {
    let events = read_recent_lines("web", Path::new("/not/real/events.jsonl"), 4);
    assert!(events.is_empty());
}

#[test]
fn provider_metrics_aggregate_usage_and_errors() {
    let first = ProviderRuntimeEventRecord::new(
        "event-1",
        "anthropic",
        "direct",
        "request_succeeded",
        "info",
        "now",
    )
    .with_model("claude")
    .with_redacted_json(serde_json::json!({
        "usage": { "input_count": 100, "output_count": 40 },
        "latency_ms": 250,
        "cost_usd": 0.002
    }));
    let second = ProviderRuntimeEventRecord::new(
        "event-2",
        "anthropic",
        "direct",
        "request_failed",
        "warn",
        "later",
    )
    .with_retry_count(2)
    .with_redacted_json(serde_json::json!({
        "usage": { "input_count": 20, "output_count": 0 },
        "latency_ms": 800
    }));
    let metrics = provider_metrics(&[first, second]);
    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].request_count, 2);
    assert_eq!(metrics[0].error_count, 1);
    assert_eq!(metrics[0].retry_count, 2);
    assert_eq!(metrics[0].input_tokens, 120);
    assert_eq!(metrics[0].output_tokens, 40);
    assert_eq!(metrics[0].latency_ms_p95, 800);
}
