//! Retry contention while writers progress; a stall ends as a typed pause.
//!
//! Raw SQLite busy and a fail-fast write lock that another handle holds are
//! both retried with backoff. There is no attempt limit and no total clock:
//! [`Contention`] stops the retries only after one full no-progress window
//! (`CozoGuardConfig::busy_wait`), and returns `StoreBusy`. An acquisition
//! pause that is already typed is returned at once and never restarted.
use std::thread;

use anyhow::{Result, anyhow};
use cozo::ScriptMutability;

use crate::contention::Contention;
use crate::{CozoGuardConfig, is_retryable_cozo_error, run_guarded_once};

pub fn run_guarded<T>(
    context: &str,
    mutability: ScriptMutability,
    config: &CozoGuardConfig,
    mut run: impl FnMut() -> Result<T>,
) -> Result<T> {
    let mut contention = Contention::new();
    loop {
        match run_guarded_once(context, mutability, config, &mut run) {
            Ok(value) => return Ok(value),
            Err(error) => match retry_or_end(context, config, &mut contention, error) {
                Retry::After(backoff) => thread::sleep(backoff),
                Retry::End(error) => return Err(error),
            },
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
    let context = context.to_string();
    let config = config.clone();
    let mut run = run;
    let mut contention = Contention::new();
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
            Err(error) => match retry_or_end(&context, &config, &mut contention, error) {
                Retry::After(backoff) => tokio::time::sleep(backoff).await,
                Retry::End(error) => return Err(error),
            },
        }
    }
}

enum Retry {
    After(std::time::Duration),
    End(anyhow::Error),
}

fn retry_or_end(
    context: &str,
    config: &CozoGuardConfig,
    contention: &mut Contention,
    error: anyhow::Error,
) -> Retry {
    // A typed acquisition pause is authoritative and keeps its category.
    if crate::StoreBusy::find(error.as_ref()).is_some() {
        return Retry::End(error);
    }
    let last_error = format!("{error:#}");
    #[cfg(any(test, feature = "test-support"))]
    if crate::is_store_contention(&last_error) {
        crate::busy_observer::notify(context, &last_error);
    }
    if retryable_cause(&error) && is_retryable_cozo_error(&last_error) {
        return match contention.next(context, config, &last_error) {
            Ok(backoff) => Retry::After(backoff),
            Err(pause) => Retry::End(pause),
        };
    }
    let message = format!("{context}: {error:#}");
    Retry::End(error.context(message))
}

// An I/O cause is authoritative. A pathname or outer context saying "locked"
// cannot turn ENOLCK, EOPNOTSUPP or policy denial into contention.
fn retryable_cause(error: &anyhow::Error) -> bool {
    error
        .chain()
        .filter_map(|e| e.downcast_ref::<std::io::Error>())
        .all(|e| e.kind() == std::io::ErrorKind::WouldBlock)
}
