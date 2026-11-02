use std::time::{Duration, Instant};

// Allow overloaded hosts and the native event queue time to recover, including
// the discovery debounce/config reload, but fail a missed event within 30s.
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

pub fn wait_until(observation: &str, mut observed: impl FnMut() -> bool) {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    loop {
        assert!(
            Instant::now() < deadline,
            "timed out after {EVENT_TIMEOUT:?}: {observation}"
        );
        if observed() {
            return;
        }
        // Only pace polling; success always requires the observable state.
        std::thread::sleep(
            Duration::from_millis(20).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}
