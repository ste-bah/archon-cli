//! Provider-attempt accounting shared by the dispatcher and the executor.
//!
//! A request that has not opened yet has an unknown outcome. Hold its host
//! clocks until response headers arrive. A successful open followed by silence spends the ordinary
//! no-progress window. Recovery never invents novel author activity.
use crate::subagent_activity::ActivityPause;
use crate::subagent_dispatch_clock::ClockPause;
use std::future::Future;

tokio::task_local! { static ACTIVE: (); }

pub fn enabled() -> bool {
    ACTIVE.try_with(|_| ()).is_ok()
}

pub async fn scope<T>(work: impl Future<Output = T>) -> T {
    ACTIVE.scope((), work).await
}

pub async fn inherit<T>(active: bool, work: impl Future<Output = T>) -> T {
    if active {
        scope(work).await
    } else {
        work.await
    }
}

pub struct ProviderAttempt {
    _paused: Vec<ClockPause>,
    activity: Option<ActivityPause>,
}

impl ProviderAttempt {
    pub fn start(agent_id: &str) -> Option<Self> {
        if !enabled() {
            return None;
        }
        let clocks = crate::subagent_dispatch_clock::current_for(agent_id)
            .map(|session| session.clocks)
            .unwrap_or_default();
        let paused = clocks
            .iter()
            .map(|clock| {
                clock.mark_provider_accounted();
                clock.pause_for_slot()
            })
            .collect();
        Some(Self {
            _paused: paused,
            activity: crate::subagent_activity::current().map(|clock| clock.pause_for_provider()),
        })
    }

    pub fn start_current() -> Option<Self> {
        let session = crate::subagent_dispatch_clock::current_session()?;
        Self::start(&session.agent_id)
    }
}

impl Drop for ProviderAttempt {
    fn drop(&mut self) {
        if let Some(activity) = &mut self.activity {
            activity.exclude_elapsed(true);
        }
    }
}

/// Exclude only the read gap that ended with a typed operational error. Earlier
/// partial output and its no-progress accounting survive the failed receive.
pub fn exclude_silence(elapsed: std::time::Duration) {
    if !enabled() {
        return;
    }
    if let Some(session) = crate::subagent_dispatch_clock::current_session() {
        for clock in session.clocks {
            clock.refund(elapsed);
        }
    }
    if let Some(clock) = crate::subagent_activity::current() {
        clock.exclude_silence(elapsed);
    }
}
