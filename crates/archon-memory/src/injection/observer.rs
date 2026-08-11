//! Watching what injection actually put in front of the model.
//!
//! Consolidation writes semantic memories. Whether anything ever reads them is a
//! separate question, and the one the R4 promotion gate asks: a consolidated
//! memory nobody recalls is prompt budget spent on tidiness. Answering it needs
//! an observation at the moment a prompt is built, which is here.
//!
//! # Why a process-wide sink rather than a handle on the injector
//!
//! Two reasons, and the second is the decisive one.
//!
//! The observer has to reach the cognitive metric store, and `archon-memory` is
//! a leaf crate that cannot depend on the crate that owns it. So the observer
//! must be implemented above both and handed down.
//!
//! And it cannot be handed to an injector instance, because injector instances
//! do not survive. The agent replaces the whole `MemoryInjector` when its cache
//! cannot be invalidated in place, so anything attached to one would be dropped
//! on the next invalidation — and the symptom would be a metric that works until
//! the store changes, then silently stops. A sink registered against the process
//! outlives every rebuild.
//!
//! This is the shape telemetry sinks normally take, for the same reason: the
//! thing being observed should not have to know it is being observed, or carry a
//! handle it has no use for.
//!
//! # It must never affect the injection
//!
//! [`notify`] swallows everything. An observer that is slow, wrong, or panicking
//! must not change which memories reach the prompt — the measurement exists to
//! describe the behaviour, and a measurement that alters it describes itself.

use std::sync::{Arc, OnceLock, RwLock};

use crate::types::Memory;

/// What one injection did.
///
/// `injected` is a prefix of `recalled`: the formatter walks the ranked recall
/// and stops at the token budget, so everything after the cut was retrieved and
/// then dropped. Both are carried because "not recalled at all" and "recalled
/// but crowded out" are different failures and only one of them is about
/// relevance.
#[derive(Debug, Clone, Copy)]
pub struct InjectionOutcome<'a> {
    /// Identity of the prompt context this injection was built for. Stable for
    /// a repeated injection of the same context, which is what lets a metric
    /// store recognise a replay instead of counting one prompt twice.
    pub context_hash: u64,
    /// Everything recall returned, ranked.
    pub recalled: &'a [Memory],
    /// The prefix that fitted the token budget and reached the prompt.
    pub injected: &'a [Memory],
    /// Whether this injection was served from the injector's cache.
    ///
    /// Reported rather than suppressed: a cached block still goes into the
    /// prompt, so it is still an injection. Recording it is how a memory that is
    /// used on every turn of a long conversation reads as used, rather than as
    /// used once.
    pub from_cache: bool,
}

/// Something that wants to know what injection put in front of the model.
pub trait InjectionObserver: Send + Sync {
    fn observed(&self, outcome: &InjectionOutcome<'_>);
}

fn slot() -> &'static RwLock<Option<Arc<dyn InjectionObserver>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<dyn InjectionObserver>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Install the process-wide injection observer, replacing any previous one.
pub fn set_injection_observer(observer: Arc<dyn InjectionObserver>) {
    if let Ok(mut guard) = slot().write() {
        *guard = Some(observer);
    }
}

/// Remove the observer. Injection then reports to nobody, which is the default.
pub fn clear_injection_observer() {
    if let Ok(mut guard) = slot().write() {
        *guard = None;
    }
}

/// Whether an observer is installed. Cheap enough to call per injection.
pub fn has_injection_observer() -> bool {
    slot().read().map(|guard| guard.is_some()).unwrap_or(false)
}

/// Tell the observer what happened, if there is one.
///
/// Every failure mode is swallowed: no observer, a poisoned lock, or an observer
/// that panics. A prompt must not change shape because telemetry had a bad day,
/// and an injection that returned successfully must not become an error after
/// the fact.
pub fn notify(outcome: &InjectionOutcome<'_>) {
    let Ok(guard) = slot().read() else {
        return;
    };
    let Some(observer) = guard.as_ref() else {
        return;
    };
    // The observer reaches a database and is written by us, not by a plugin,
    // but a panic here would poison the injector mutex the agent shares across
    // turns -- so the blast radius is contained rather than trusted.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        observer.observed(outcome);
    }));
    if result.is_err() {
        tracing::warn!("memory injection observer panicked; the injection is unaffected");
    }
}

#[cfg(test)]
#[path = "observer_tests.rs"]
mod observer_tests;
