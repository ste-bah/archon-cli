//! Retry contention without a total attempt cap; explicit pauses stay typed.
use std::thread;

use anyhow::{Result, anyhow};
use cozo::ScriptMutability;

use crate::retry::{backoff_steps, retry_backoff};
use crate::{CozoGuardConfig, run_guarded_once};

pub fn run_guarded<T>(
    context: &str,
    mutability: ScriptMutability,
    config: &CozoGuardConfig,
    mut run: impl FnMut() -> Result<T>,
) -> Result<T> {
    let ramp_steps = backoff_steps(config);

    let mut step = 0;
    loop {
        match run_guarded_once(context, mutability, config, &mut run) {
            Ok(value) => return Ok(value),
            Err(error) => {
                // A typed acquisition pause is authoritative and must retain
                // its category. Raw SQLite busy says nothing about progress:
                // even a writer completing no-op statements may not change
                // its files. Never infer a stall from total attempts or time.
                if crate::StoreBusy::find(error.as_ref()).is_some() {
                    return Err(error);
                }
                let last_error = format!("{error:#}");
                #[cfg(feature = "test-support")]
                if crate::is_store_contention(&last_error) {
                    crate::busy_observer::notify(context, &last_error);
                }
                if let Some(backoff) = retry_backoff(context, config, step, &last_error) {
                    step = step.saturating_add(1).min(ramp_steps - 1);
                    thread::sleep(backoff);
                    continue;
                }
                let message = format!("{context}: {error:#}");
                return Err(error.context(message));
            }
        }
    }
}

pub async fn run_guarded_async<T, Run>(
    context: &str,
    mutability: ScriptMutability,
    config: &CozoGuardConfig,
    run: Run,
) -> Result<T>
where
    T: Send + 'static,
    Run: FnMut() -> Result<T> + Send + 'static,
{
    let ramp_steps = backoff_steps(config);
    let context = context.to_string();
    let config = config.clone();
    let mut run = run;

    let mut step = 0;
    loop {
        let attempt_context = context.clone();
        let attempt_config = config.clone();
        let attempt_result = tokio::task::spawn_blocking(move || {
            let result = run_guarded_once(&attempt_context, mutability, &attempt_config, &mut run);
            (run, result)
        })
        .await
        .map_err(|error| anyhow!("{context}: guarded operation task failed: {error}"))?;
        run = attempt_result.0;

        match attempt_result.1 {
            Ok(value) => return Ok(value),
            Err(error) => {
                // A typed acquisition pause is authoritative and must retain
                // its category. Raw SQLite busy says nothing about progress:
                // even a writer completing no-op statements may not change
                // its files. Never infer a stall from total attempts or time.
                if crate::StoreBusy::find(error.as_ref()).is_some() {
                    return Err(error);
                }
                let last_error = format!("{error:#}");
                #[cfg(feature = "test-support")]
                if crate::is_store_contention(&last_error) {
                    crate::busy_observer::notify(&context, &last_error);
                }
                if let Some(backoff) = retry_backoff(&context, &config, step, &last_error) {
                    step = step.saturating_add(1).min(ramp_steps - 1);
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                let message = format!("{context}: {error:#}");
                return Err(error.context(message));
            }
        }
    }
}
