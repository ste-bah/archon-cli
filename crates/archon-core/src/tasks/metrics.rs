use std::sync::atomic::{AtomicU64, Ordering};
use dashmap::DashMap;

/// Atomic counters for task execution metrics.
/// Thread-safe, lock-free reads. Prometheus text format export.
pub struct MetricsRegistry {
    pub(crate) tasks_started_total: AtomicU64,
    pub(crate) tasks_finished_total: AtomicU64,
    pub(crate) tasks_failed_total: AtomicU64,
    queue_depths: DashMap<String, AtomicU64>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            tasks_started_total: AtomicU64::new(0),
            tasks_finished_total: AtomicU64::new(0),
            tasks_failed_total: AtomicU64::new(0),
            queue_depths: DashMap::new(),
        }
    }

    pub fn inc_started(&self) {
        self.tasks_started_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_finished(&self) {
        self.tasks_finished_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_failed(&self) {
        self.tasks_failed_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_queue_depth(&self, agent: &str, depth: u64) {
        self.queue_depths
            .entry(agent.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .store(depth, Ordering::Relaxed);
    }

    /// Read the current value of `tasks_started_total`.
    pub fn started_total(&self) -> u64 {
        self.tasks_started_total.load(Ordering::Relaxed)
    }

    /// Read the current value of `tasks_finished_total`.
    pub fn finished_total(&self) -> u64 {
        self.tasks_finished_total.load(Ordering::Relaxed)
    }

    /// Read the current value of `tasks_failed_total`.
    pub fn failed_total(&self) -> u64 {
        self.tasks_failed_total.load(Ordering::Relaxed)
    }

    /// Export all metrics in Prometheus text exposition format.
    pub fn export_prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "tasks_started_total {}\n",
            self.tasks_started_total.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "tasks_finished_total {}\n",
            self.tasks_finished_total.load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "tasks_failed_total {}\n",
            self.tasks_failed_total.load(Ordering::Relaxed)
        ));

        // Sort agent names for deterministic output.
        let mut agents: Vec<_> = self
            .queue_depths
            .iter()
            .map(|r| (r.key().clone(), r.value().load(Ordering::Relaxed)))
            .collect();
        agents.sort_by(|a, b| a.0.cmp(&b.0));

        for (agent, depth) in agents {
            out.push_str(&format!(
                "queue_depth{{agent=\"{}\"}} {}\n",
                agent, depth
            ));
        }

        out
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}
