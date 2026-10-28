//! Availability recovery before a capability's single durable outcome.
use std::future::Future;
use std::time::Duration;

use anyhow::Result;
use archon_workflow::WorkflowStore;

use super::workflow_host_command_operational::{
    PausedRunAuthorization, require_run_owned_authorized,
};

const INITIAL_DELAY: Duration = Duration::from_millis(500);
const MAX_DELAY: Duration = Duration::from_secs(8);

/// Existing ownership/control checks also authorize read-only paused reverification.
pub(crate) type Control<'a> = (
    &'a WorkflowStore,
    &'a str,
    u64,
    Option<&'a PausedRunAuthorization>,
);

pub(crate) async fn retry<T, F, Fut>(capability: &str, control: Control<'_>, work: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    retry_waiting(capability, work, |delay| wait(control, delay)).await
}

/// The same backoff for recovery admission, whose existing control check
/// permits its initial paused state while it holds the executor lease.
pub(crate) async fn retry_waiting<T, F, Fut, W, Waiting>(
    capability: &str,
    mut work: F,
    mut wait: W,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
    W: FnMut(Duration) -> Waiting,
    Waiting: Future<Output = Result<()>>,
{
    let mut attempt = 1u64;
    let mut delay = INITIAL_DELAY;
    loop {
        match work().await {
            Ok(value) => return Ok(value),
            Err(error) if transient(&error) => {
                tracing::warn!(
                    component = "host_command",
                    capability,
                    attempt,
                    next_delay_ms = delay.as_millis() as u64,
                    cause_class = "TRANSIENT",
                    "retrying unavailable host capability"
                );
                wait(delay).await?;
                attempt = attempt.saturating_add(1);
                delay = (delay * 2).min(MAX_DELAY);
            }
            Err(error) => {
                return Err(error);
            }
        }
    }
}

/// Only the delay is abandoned: the preceding child's teardown and staging
/// seal have finished. Reuse the supervisor's ownership check and polling cadence.
async fn wait(
    (store, run_id, generation, authorization): Control<'_>,
    delay: Duration,
) -> Result<()> {
    controlled((store, run_id, generation, authorization), async {
        tokio::time::sleep(delay).await;
        Ok(())
    })
    .await
}

pub(crate) async fn controlled<T>(
    (store, run_id, generation, authorization): Control<'_>,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    controlled_by(
        || {
            require_run_owned_authorized(store, run_id, generation, authorization)
                .map_err(Into::into)
        },
        work,
    )
    .await
}

pub(crate) async fn controlled_by<T>(
    check: impl Fn() -> Result<()>,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::pin!(work);
    let mut poll = tokio::time::interval(Duration::from_millis(100));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = poll.tick() => {
                if let Err(error) = check() {
                    if !transient(&error) { return Err(error); }
                }
            },
            result = &mut work => return result,
        }
    }
}

pub(crate) fn transient(error: &anyhow::Error) -> bool {
    super::workflow_host_fault::classify(error) == super::workflow_host_fault::Class::Transient
}
