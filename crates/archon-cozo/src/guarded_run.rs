//! Retry transient contention; exhausted call windows remain resumable.
use std::thread;

use anyhow::{Result, anyhow};
use cozo::ScriptMutability;

use crate::retry::{normalized_attempts, retry_backoff};
use crate::{CozoGuardConfig, run_guarded_once};

pub fn run_guarded<T>(
    context: &str,
    mutability: ScriptMutability,
    config: &CozoGuardConfig,
    mut run: impl FnMut() -> Result<T>,
) -> Result<T> {
    let attempts = normalized_attempts(config);

    for attempt in 0..attempts {
        match run_guarded_once(context, mutability, config, &mut run) {
            Ok(value) => return Ok(value),
            Err(error) => {
                let last_error = format!("{error:#}");
                #[cfg(feature = "test-support")]
                if crate::is_store_contention(&last_error) {
                    crate::busy_observer::notify(context, &last_error);
                }
                // A completed acquisition window is already an explicit busy
                // response. Let the caller resume; don't restart it internally.
                if error.is::<crate::StoreBusy>() {
                    return Err(error);
                }
                if let Some(backoff) =
                    retry_backoff(context, config, attempt, attempts, &last_error)
                {
                    thread::sleep(backoff);
                    continue;
                }
                return Err(crate::busy::guarded_error(context, attempts, last_error));
            }
        }
    }

    unreachable!("a guarded retry loop always returns from an attempt")
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
    let attempts = normalized_attempts(config);
    let context = context.to_string();
    let config = config.clone();
    let mut run = run;

    for attempt in 0..attempts {
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
                let last_error = format!("{error:#}");
                #[cfg(feature = "test-support")]
                if crate::is_store_contention(&last_error) {
                    crate::busy_observer::notify(&context, &last_error);
                }
                // A completed acquisition window is already an explicit busy
                // response. Let the caller resume; don't restart it internally.
                if error.is::<crate::StoreBusy>() {
                    return Err(error);
                }
                if let Some(backoff) =
                    retry_backoff(&context, &config, attempt, attempts, &last_error)
                {
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                return Err(crate::busy::guarded_error(&context, attempts, last_error));
            }
        }
    }

    unreachable!("a guarded retry loop always returns from an attempt")
}
