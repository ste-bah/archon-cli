//! metrics — MetricsRecorder + latency/memory helpers for eventloop benches.

use hdrhistogram::Histogram;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct DrainSample {
    pub t: Instant,
    pub batch: usize,
    pub backlog: usize,
    pub mem_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub total_events: u64,
    pub peak_backlog: usize,
    pub peak_mem_bytes: usize,
    pub throughput_eps: f64,
}

pub struct MetricsRecorder {
    hist: Histogram<u64>,
    drain_samples: Vec<DrainSample>,
    total_events: u64,
    peak_backlog: usize,
    peak_mem_bytes: usize,
    start: Option<Instant>,
    last_seen: Option<Instant>,
}

impl MetricsRecorder {
    pub fn new() -> Self {
        // hdrhistogram: 1us..60s range, 3 sig figs
        let hist =
            Histogram::<u64>::new_with_bounds(1, 60_000_000, 3).expect("histogram bounds valid");
        Self {
            hist,
            drain_samples: Vec::new(),
            total_events: 0,
            peak_backlog: 0,
            peak_mem_bytes: 0,
            start: None,
            last_seen: None,
        }
    }

    /// Observe a drain of `batch` events that were enqueued at `send_ts`.
    /// Records latency of each event in the batch against `send_ts` (simplification:
    /// treats all events in a batch as having the same send_ts; callers can loop).
    pub fn observe_drain(&mut self, batch: usize, send_ts: Instant) {
        let now = Instant::now();
        if self.start.is_none() {
            self.start = Some(send_ts);
        }
        self.last_seen = Some(now);
        let lat_us = now.saturating_duration_since(send_ts).as_micros() as u64;
        let lat_us = lat_us.max(1); // hdrhistogram lowest bound is 1
        for _ in 0..batch {
            let _ = self.hist.record(lat_us);
        }
        self.total_events += batch as u64;
    }

    pub fn sample_backlog(&mut self, backlog: usize, mem_bytes: usize) {
        let now = Instant::now();
        self.drain_samples.push(DrainSample {
            t: now,
            batch: 0,
            backlog,
            mem_bytes,
        });
        if backlog > self.peak_backlog {
            self.peak_backlog = backlog;
        }
        if mem_bytes > self.peak_mem_bytes {
            self.peak_mem_bytes = mem_bytes;
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let elapsed = match (self.start, self.last_seen) {
            (Some(s), Some(e)) => e.saturating_duration_since(s),
            _ => Duration::ZERO,
        };
        let secs = elapsed.as_secs_f64();
        let throughput_eps = if secs > 0.0 {
            self.total_events as f64 / secs
        } else {
            0.0
        };
        MetricsSnapshot {
            p50_us: self.hist.value_at_quantile(0.50),
            p95_us: self.hist.value_at_quantile(0.95),
            p99_us: self.hist.value_at_quantile(0.99),
            total_events: self.total_events,
            peak_backlog: self.peak_backlog,
            peak_mem_bytes: self.peak_mem_bytes,
            throughput_eps,
        }
    }
}

impl Default for MetricsRecorder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct GrowthError {
    pub message: String,
    pub observed_mb_per_1k: f64,
    pub limit_mb_per_1k: f64,
}

/// Assert that memory growth across `samples` stays within `max_mb_per_1k_events`.
/// `samples` is a slice of (Instant, mem_bytes) pairs; growth is computed from
/// first to last sample divided by *number of samples* as a proxy for events.
/// The caller is responsible for passing one sample per 1000 events (or similar scale).
pub fn assert_linear_memory_growth(
    samples: &[(Instant, usize)],
    max_mb_per_1k_events: f64,
) -> Result<(), GrowthError> {
    if samples.len() < 2 {
        return Ok(());
    }
    let first_mem = samples.first().unwrap().1 as f64;
    let last_mem = samples.last().unwrap().1 as f64;
    let delta_bytes = last_mem - first_mem;
    let delta_mb = delta_bytes / (1024.0 * 1024.0);
    // Each sample is assumed to represent 1000 events (phase-2 contract).
    let n_k_events = (samples.len() - 1) as f64;
    let observed = delta_mb / n_k_events.max(1.0);
    if observed > max_mb_per_1k_events {
        return Err(GrowthError {
            message: format!(
                "memory grew {:.2} MB per 1k events, limit {:.2}",
                observed, max_mb_per_1k_events
            ),
            observed_mb_per_1k: observed,
            limit_mb_per_1k: max_mb_per_1k_events,
        });
    }
    Ok(())
}
