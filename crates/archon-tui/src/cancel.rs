//! Cooperative-cancellation yield gate for hot loops.
//!
//! Used inside hot sync loops that have been retrofitted to an async
//! context so that a cancelled `JoinHandle` gets a chance to unwind
//! between iterations. At present no call sites in `archon-tui` exercise
//! this type (all existing hot loops are sync and not cancellable
//! regardless — see TASK-TUI-108 skip-path comments in `markdown.rs`
//! and `syntax.rs`), but the primitive is kept here so future async
//! render paths can adopt it without reinventing it.

/// Cooperative-cancellation yield gate. Call `tick().await` inside hot
/// sync loops that have been retrofitted to an async context; every
/// `period` calls, it yields to the tokio runtime so a cancelled
/// JoinHandle gets a chance to unwind.
///
/// `period == 0` is a noop (explicit short-circuit before counter
/// increment — a naive `counter >= period` check would yield every
/// single call when period is 0, opposite of the intended contract).
pub struct YieldGate {
    counter: u32,
    period: u32,
}

impl YieldGate {
    /// Create a new yield gate. A `period` of 0 disables yielding.
    pub fn new(period: u32) -> Self {
        Self { counter: 0, period }
    }

    /// Increment the internal counter; every `period` calls, yield to
    /// the tokio runtime. A `period` of 0 is a noop.
    pub async fn tick(&mut self) {
        if self.period == 0 {
            return;
        }
        self.counter += 1;
        if self.counter >= self.period {
            self.counter = 0;
            tokio::task::yield_now().await;
        }
    }

    /// Test-only accessor for the internal counter. Hidden from the
    /// public surface to keep the API minimal.
    #[cfg(test)]
    fn counter(&self) -> u32 {
        self.counter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With `period = 64`, 200 `tick()` calls should produce exactly
    /// 3 yield/reset events (at 64, 128, 192) leaving the counter at
    /// `200 - 192 = 8`. This pins the every-N yielding behavior without
    /// depending on wall-clock observation of the runtime.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_yield_gate_yields_every_n() {
        let mut gate = YieldGate::new(64);
        for _ in 0..200 {
            gate.tick().await;
        }
        assert_eq!(
            gate.counter(),
            8,
            "after 200 ticks with period 64, counter should be 200 % 64 = 8",
        );
    }

    /// With `period = 0`, `tick()` must never touch the counter and
    /// must never yield. This pins the short-circuit fix for the spec
    /// skeleton bug (see D2 in TASK-TUI-108 deviation block).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_yield_gate_zero_period_is_noop() {
        let mut gate = YieldGate::new(0);
        for _ in 0..1000 {
            gate.tick().await;
            assert_eq!(
                gate.counter(),
                0,
                "period=0 must short-circuit before incrementing counter",
            );
        }
        assert_eq!(gate.counter(), 0);
    }
}
