use std::time::Duration;

#[derive(Debug, Clone)]
pub(crate) struct AdaptiveBatchController {
    current: usize,
    min: usize,
    max: usize,
    enabled: bool,
}

impl AdaptiveBatchController {
    pub(crate) fn from_initial(initial: usize) -> Self {
        let initial = initial.max(1);
        let enabled = std::env::var("ARCHON_DOCS_ADAPTIVE_BATCHING")
            .map(|value| value != "0" && value != "false")
            .unwrap_or(true);
        let min = env_usize("ARCHON_DOCS_BATCH_MIN").unwrap_or_else(|| (initial / 4).max(1));
        let max = env_usize("ARCHON_DOCS_BATCH_MAX").unwrap_or_else(|| (initial * 4).max(initial));
        Self {
            current: initial.clamp(min, max),
            min,
            max,
            enabled,
        }
    }

    pub(crate) fn next_size(&self) -> usize {
        self.current
    }

    pub(crate) fn observe_success(&mut self, elapsed: Duration, chunk_count: usize) {
        if !self.enabled || chunk_count == 0 || self.current >= self.max {
            return;
        }
        let millis_per_chunk = elapsed.as_millis() / chunk_count as u128;
        if millis_per_chunk < 350 {
            self.current = (self.current + growth_step(self.current)).min(self.max);
        }
    }

    pub(crate) fn observe_failure(&mut self) {
        if self.enabled {
            self.current = (self.current / 2).max(self.min);
        }
    }
}

fn growth_step(current: usize) -> usize {
    (current / 4).max(1)
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controller_grows_on_fast_success_and_shrinks_on_failure() {
        let mut controller = AdaptiveBatchController::from_initial(8);
        controller.observe_success(Duration::from_millis(80), 8);
        assert!(controller.next_size() > 8);
        controller.observe_failure();
        assert!(controller.next_size() >= 2);
    }
}
