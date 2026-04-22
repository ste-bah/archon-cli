//! AGS-OBS-902 Gate 5 live-smoke test.
//!
//! End-to-end exercise of the Prometheus /metrics HTTP exporter:
//!   1. Bind a real loopback `TcpListener` on an OS-assigned port.
//!   2. Spawn `serve_metrics_on` over it.
//!   3. Fetch `/metrics` with reqwest.
//!   4. Assert status, Content-Type, and that the body parses into the five
//!      metrics declared by `format_prometheus`.
//!
//! This is the production HTTP path — not a format-only unit test. If the
//! router, listener adapter, or handler regresses, this fails.
//!
//! Runs on default `cargo test -p archon-tui`; no feature flag.
//!
//! Evidence for gate 5 of TASK-AGS-OBS-902.

use std::sync::Arc;
use std::time::Duration;

use archon_tui::observability::{ChannelMetrics, serve_metrics_on};

#[tokio::test]
async fn metrics_endpoint_serves_five_metrics_over_real_http() {
    // Bind an ephemeral loopback port synchronously so the OS assigns a free
    // one; mirror the production `spawn_metrics_exporter` path that does
    // `std::net::TcpListener::bind` + `set_nonblocking(true)` + `from_std`.
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral loopback port");
    std_listener
        .set_nonblocking(true)
        .expect("set_nonblocking on listener");
    let addr = std_listener.local_addr().expect("local_addr");
    let listener = tokio::net::TcpListener::from_std(std_listener)
        .expect("promote std listener to tokio");

    let metrics = Arc::new(ChannelMetrics::new());
    // Seed the gauges/counters so the body has non-zero samples to assert on.
    metrics.record_sent();
    metrics.record_sent();
    metrics.record_drained(1);
    metrics.record_latency_ms(42);

    let serve_metrics = Arc::clone(&metrics);
    let serve_handle = tokio::spawn(async move {
        serve_metrics_on(listener, serve_metrics).await
    });

    // Give the server a beat to accept on the new listener. Poll a few times
    // rather than sleep-blindly so we don't race on loaded CI.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client");
    let url = format!("http://{}/metrics", addr);

    let mut last_err: Option<String> = None;
    let mut resp = None;
    for _ in 0..20 {
        match client.get(&url).send().await {
            Ok(r) => {
                resp = Some(r);
                break;
            }
            Err(e) => {
                last_err = Some(e.to_string());
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
    let resp = resp.unwrap_or_else(|| {
        panic!(
            "metrics endpoint never accepted a request; last error: {:?}",
            last_err
        )
    });

    assert!(
        resp.status().is_success(),
        "metrics endpoint returned non-2xx: {}",
        resp.status()
    );

    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        ctype.starts_with("text/plain") && ctype.contains("version=0.0.4"),
        "Content-Type must be Prometheus text-format v0.0.4; got {:?}",
        ctype
    );

    let body = resp.text().await.expect("read body");

    // Structural check: every declared metric name is present with a TYPE
    // line. We don't re-implement the full parser here — the lib tests
    // already enforce the exposition shape — but we assert the surface
    // that HTTP clients will see.
    for (name, ty) in [
        ("archon_tui_channel_backlog_depth", "gauge"),
        ("archon_tui_channel_total_sent", "counter"),
        ("archon_tui_channel_total_drained", "counter"),
        ("archon_tui_channel_max_batch_size", "gauge"),
        ("archon_tui_channel_p95_send_to_render_ms", "gauge"),
    ] {
        let type_line = format!("# TYPE {} {}", name, ty);
        assert!(
            body.contains(&type_line),
            "missing `{}` in body:\n{}",
            type_line,
            body,
        );
    }

    // Shut the server down cleanly so the test task fleet doesn't leak.
    serve_handle.abort();
    let _ = serve_handle.await;
}
