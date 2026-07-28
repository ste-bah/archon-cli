use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Approximate count of `TuiEvent`s queued in the render-loop input channel.
pub static TUI_EVENT_PENDING: AtomicUsize = AtomicUsize::new(0);

/// Heap allocation bytes currently retained by queued `TuiEvent` payloads.
pub static TUI_EVENT_PENDING_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Largest observed [`TUI_EVENT_PENDING_BYTES`] value since process start.
pub static TUI_EVENT_PENDING_BYTE_HIGH_WATER: AtomicUsize = AtomicUsize::new(0);

/// Total TUI events rejected because one bounded frame still exceeded the
/// per-frame payload limit.
pub static TUI_EVENT_OVERSIZED_REJECTED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Total oversized convenience metadata fields omitted from otherwise lossless
/// primary events.
pub static TUI_EVENT_OVERSIZED_METADATA_REJECTED_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Total nanoseconds async TUI producers spent waiting for queue capacity.
pub static TUI_EVENT_BLOCKED_SEND_DURATION_NS: AtomicU64 = AtomicU64::new(0);

/// Total synchronous sends rejected because the bounded queue was full.
pub static TUI_EVENT_FULL_SEND_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Total sends rejected because the TUI receiver was closed.
pub static TUI_EVENT_CLOSED_SEND_FAILURES: AtomicU64 = AtomicU64::new(0);

pub fn record_tui_event_enqueued(bytes: usize) {
    TUI_EVENT_PENDING.fetch_add(1, Ordering::Relaxed);
    add_pending_bytes(bytes);
}

pub fn record_tui_event_coalesced_bytes(bytes: usize) {
    add_pending_bytes(bytes);
}

pub fn record_tui_event_dequeued(bytes: usize) {
    decrement_pending();
    subtract_pending_bytes(bytes);
}

pub fn record_tui_event_discarded(bytes: usize) {
    decrement_pending();
    subtract_pending_bytes(bytes);
}

pub fn tui_event_pending_count() -> usize {
    TUI_EVENT_PENDING.load(Ordering::Relaxed)
}

pub fn tui_event_pending_bytes() -> usize {
    TUI_EVENT_PENDING_BYTES.load(Ordering::Relaxed)
}

pub fn tui_event_pending_byte_high_water() -> usize {
    TUI_EVENT_PENDING_BYTE_HIGH_WATER.load(Ordering::Relaxed)
}

pub fn record_tui_event_oversized_rejected() {
    TUI_EVENT_OVERSIZED_REJECTED_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn tui_event_oversized_rejected_count() -> u64 {
    TUI_EVENT_OVERSIZED_REJECTED_TOTAL.load(Ordering::Relaxed)
}

pub fn record_tui_event_oversized_metadata_rejected() {
    TUI_EVENT_OVERSIZED_METADATA_REJECTED_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub fn tui_event_oversized_metadata_rejected_count() -> u64 {
    TUI_EVENT_OVERSIZED_METADATA_REJECTED_TOTAL.load(Ordering::Relaxed)
}

pub fn record_tui_event_blocked_send(duration: std::time::Duration) {
    TUI_EVENT_BLOCKED_SEND_DURATION_NS.fetch_add(
        duration.as_nanos().min(u128::from(u64::MAX)) as u64,
        Ordering::Relaxed,
    );
}

pub fn tui_event_blocked_send_duration_ns() -> u64 {
    TUI_EVENT_BLOCKED_SEND_DURATION_NS.load(Ordering::Relaxed)
}

pub fn record_tui_event_full_send_failure() {
    TUI_EVENT_FULL_SEND_FAILURES.fetch_add(1, Ordering::Relaxed);
}

pub fn tui_event_full_send_failure_count() -> u64 {
    TUI_EVENT_FULL_SEND_FAILURES.load(Ordering::Relaxed)
}

pub fn record_tui_event_closed_send_failure() {
    TUI_EVENT_CLOSED_SEND_FAILURES.fetch_add(1, Ordering::Relaxed);
}

pub fn tui_event_closed_send_failure_count() -> u64 {
    TUI_EVENT_CLOSED_SEND_FAILURES.load(Ordering::Relaxed)
}

pub fn reset_for_tests() {
    TUI_EVENT_PENDING.store(0, Ordering::Relaxed);
    TUI_EVENT_PENDING_BYTES.store(0, Ordering::Relaxed);
    TUI_EVENT_PENDING_BYTE_HIGH_WATER.store(0, Ordering::Relaxed);
    TUI_EVENT_OVERSIZED_REJECTED_TOTAL.store(0, Ordering::Relaxed);
    TUI_EVENT_OVERSIZED_METADATA_REJECTED_TOTAL.store(0, Ordering::Relaxed);
    TUI_EVENT_BLOCKED_SEND_DURATION_NS.store(0, Ordering::Relaxed);
    TUI_EVENT_FULL_SEND_FAILURES.store(0, Ordering::Relaxed);
    TUI_EVENT_CLOSED_SEND_FAILURES.store(0, Ordering::Relaxed);
}

fn add_pending_bytes(bytes: usize) {
    let pending = TUI_EVENT_PENDING_BYTES
        .fetch_add(bytes, Ordering::Relaxed)
        .saturating_add(bytes);
    TUI_EVENT_PENDING_BYTE_HIGH_WATER.fetch_max(pending, Ordering::Relaxed);
}

fn subtract_pending_bytes(bytes: usize) {
    let mut current = TUI_EVENT_PENDING_BYTES.load(Ordering::Relaxed);
    loop {
        let next = current.saturating_sub(bytes);
        match TUI_EVENT_PENDING_BYTES.compare_exchange_weak(
            current,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}

fn decrement_pending() {
    let mut current = TUI_EVENT_PENDING.load(Ordering::Relaxed);
    loop {
        if current == 0 {
            return;
        }
        match TUI_EVENT_PENDING.compare_exchange_weak(
            current,
            current - 1,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}
