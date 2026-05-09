use super::*;
use archon_llm::runtime::{ProviderRateLimitWindow, RateLimitWindowKind};

#[test]
fn health_report_counts_events_and_rate_limits() {
    let status = ProviderRuntimeStatus::new("anthropic", "direct")
        .with_display_name("Anthropic")
        .with_model("claude-sonnet-4-6")
        .with_identity_status(ProviderIdentityStatus::Spoof)
        .with_health(ProviderHealthStatus::Degraded)
        .with_rate_limits(vec![
            ProviderRateLimitWindow::new("anthropic", RateLimitWindowKind::Usage)
                .with_used_percent(100.0),
        ]);
    let events = vec![
        ProviderRuntimeEventRecord::new(
            "event-1",
            "anthropic",
            "direct",
            "request_failed",
            "error",
            "2026-05-08T12:00:00Z",
        )
        .with_reason("auth_failed"),
        ProviderRuntimeEventRecord::new(
            "event-2",
            "anthropic",
            "direct",
            "request_succeeded",
            "info",
            "2026-05-08T12:01:00Z",
        ),
    ];

    let report =
        ProviderHealthReport::from_records("2026-05-08T12:02:00Z".to_string(), &[status], &events);

    assert_eq!(report.provider_count, 1);
    assert_eq!(report.providers[0].health, "degraded");
    assert_eq!(report.providers[0].identity_status, "spoof");
    assert_eq!(report.providers[0].exhausted_rate_limits, 1);
    assert_eq!(report.providers[0].event_count, 2);
    assert_eq!(report.providers[0].failure_count, 1);
    assert_eq!(
        report.providers[0]
            .last_failure
            .as_ref()
            .unwrap()
            .reason_code,
        Some("auth_failed".to_string())
    );
}
