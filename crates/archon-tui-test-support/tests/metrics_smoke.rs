use archon_tui_test_support::metrics::{
    assert_linear_memory_growth, MetricsRecorder,
};
use std::time::{Duration, Instant};

#[test]
fn recorder_new_then_observe_drain_nonzero_snapshot() {
    let mut r = MetricsRecorder::new();
    let ts = Instant::now();
    std::thread::sleep(Duration::from_millis(2));
    r.observe_drain(10, ts);
    std::thread::sleep(Duration::from_millis(2));
    r.observe_drain(5, ts);
    let snap = r.snapshot();
    assert_eq!(snap.total_events, 15);
    assert!(snap.p50_us > 0, "p50 should be nonzero, got {}", snap.p50_us);
    assert!(snap.p95_us > 0, "p95 should be nonzero, got {}", snap.p95_us);
    assert!(snap.throughput_eps >= 0.0);
}

#[test]
fn sample_backlog_tracks_peaks() {
    let mut r = MetricsRecorder::new();
    r.sample_backlog(10, 1024);
    r.sample_backlog(50, 4096);
    r.sample_backlog(20, 2048);
    let snap = r.snapshot();
    assert_eq!(snap.peak_backlog, 50);
    assert_eq!(snap.peak_mem_bytes, 4096);
}

#[test]
fn assert_linear_memory_growth_pass_on_1mb_per_1000() {
    // 5 samples, 1 MB step each => 1 MB per 1000 events, limit 2 MB => pass
    let base = Instant::now();
    let mb = 1024 * 1024;
    let samples: Vec<(Instant, usize)> = (0..5)
        .map(|i| (base + Duration::from_millis(i * 10), (i as usize) * mb))
        .collect();
    let result = assert_linear_memory_growth(&samples, 2.0);
    assert!(result.is_ok(), "expected pass, got {:?}", result);
}

#[test]
fn assert_linear_memory_growth_fail_on_1mb_per_10() {
    // 5 samples, 1 MB step each but caller claims each represents 10 events
    // so per-1k should be 100 MB. Limit 2.0 => fail.
    // Using the function's contract (samples represent 1000 events each),
    // we instead drive failure by passing a 20 MB jump between two samples.
    let base = Instant::now();
    let mb = 1024 * 1024;
    let samples: Vec<(Instant, usize)> = vec![
        (base, 0),
        (base + Duration::from_millis(10), 100 * mb),
    ];
    let result = assert_linear_memory_growth(&samples, 2.0);
    assert!(result.is_err(), "expected fail, got Ok");
}
