//! TASK-P1-4 (#189) — /metrics Prometheus endpoint E2E smoke.
//!
//! Binds a real TCP listener on 127.0.0.1:0, spawns the Prometheus
//! exporter, emits known ChannelMetrics events, scrapes /metrics over
//! HTTP, and asserts the response shape (# HELP, # TYPE lines, samples
//! with non-zero values for recorded events, UTF-8, correct
//! Content-Type).

use std::sync::Arc;
use std::time::Duration;

use archon_observability::metrics::{serve_metrics_on, ChannelMetrics};

#[tokio::test]
async fn metrics_endpoint_serves_prometheus_shape_with_live_values() {
    // Fresh metrics instance, record some events.
    let metrics = Arc::new(ChannelMetrics::new());
    metrics.record_sent();
    metrics.record_sent();
    metrics.record_drained(2);
    metrics.record_latency_ms(5);
    metrics.record_latency_ms(7);

    // Bind random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn serve task (detached; aborted at end).
    let metrics_for_server = metrics.clone();
    let server = tokio::spawn(async move {
        let _ = serve_metrics_on(listener, metrics_for_server).await;
    });

    // Let the server reach `axum::serve`.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Scrape.
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{addr}/metrics"))
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .expect("scrape failed");

    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ct.starts_with("text/plain"),
        "Content-Type must start with text/plain; got '{ct}'"
    );
    assert!(
        ct.contains("version=0.0.4"),
        "Content-Type must declare Prometheus v0.0.4; got '{ct}'"
    );

    let body = resp.text().await.expect("body");

    // Basic Prometheus text-format shape.
    assert!(body.contains("# HELP"), "response must contain # HELP lines");
    assert!(body.contains("# TYPE"), "response must contain # TYPE lines");

    // At least one "<name> <value>" sample line present and the value is
    // numeric.
    let mut sample_count = 0;
    for line in body.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(
            parts.len(),
            2,
            "sample line '{line}' must have '<name> <value>' shape"
        );
        let _: f64 = parts[1].parse().expect(
            &format!("sample value '{}' must parse as f64 (line='{line}')", parts[1]),
        );
        sample_count += 1;
    }
    assert!(sample_count >= 1, "expected at least one metric sample");

    // Live-value assertion: total_sent counter must reflect the 2
    // record_sent() calls. Prometheus metric name is whatever
    // format_prometheus emits — scan for any line containing "sent" and
    // having a non-zero value.
    let sent_line = body.lines().find(|l| {
        !l.starts_with('#')
            && l.to_lowercase().contains("sent")
            && l.split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<f64>().ok())
                .map_or(false, |v| v > 0.0)
    });
    assert!(
        sent_line.is_some(),
        "expected at least one 'sent' metric with value > 0 after 2 record_sent() calls. \
         Body:\n{body}"
    );

    // Cleanup — server is async-detached; abort.
    server.abort();
}
