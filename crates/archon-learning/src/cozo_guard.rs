use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

const MAX_ATTEMPTS: usize = 4;

static COZO_SCRIPT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn run_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows> {
    let mut last_error = String::new();

    for attempt in 0..MAX_ATTEMPTS {
        let lock = COZO_SCRIPT_LOCK.get_or_init(|| Mutex::new(()));
        let guard = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let result = catch_unwind(AssertUnwindSafe(|| {
            db.run_script(script, params.clone(), mutability)
        }));
        drop(guard);

        match result {
            Ok(Ok(rows)) => return Ok(rows),
            Ok(Err(error)) => {
                last_error = error.to_string();
                if is_retryable_cozo_error(&last_error) && attempt + 1 < MAX_ATTEMPTS {
                    backoff(attempt);
                    continue;
                }
                return Err(anyhow!("{context}: {last_error}"));
            }
            Err(payload) => {
                last_error = panic_payload_message(payload.as_ref());
                if is_retryable_cozo_error(&last_error) && attempt + 1 < MAX_ATTEMPTS {
                    backoff(attempt);
                    continue;
                }
                return Err(anyhow!(
                    "{context}: cozo sqlite backend panicked: {last_error}"
                ));
            }
        }
    }

    Err(anyhow!(
        "{context}: cozo sqlite backend stayed busy after {MAX_ATTEMPTS} attempts: {last_error}"
    ))
}

pub fn open_sqlite_guarded(path: &str, context: &str) -> Result<DbInstance> {
    let mut last_error = String::new();

    for attempt in 0..MAX_ATTEMPTS {
        let lock = COZO_SCRIPT_LOCK.get_or_init(|| Mutex::new(()));
        let guard = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let result = catch_unwind(AssertUnwindSafe(|| DbInstance::new("sqlite", path, "")));
        drop(guard);

        match result {
            Ok(Ok(db)) => return Ok(db),
            Ok(Err(error)) => {
                last_error = error.to_string();
                if is_retryable_cozo_error(&last_error) && attempt + 1 < MAX_ATTEMPTS {
                    backoff(attempt);
                    continue;
                }
                return Err(anyhow!("{context}: {last_error}"));
            }
            Err(payload) => {
                last_error = panic_payload_message(payload.as_ref());
                if is_retryable_cozo_error(&last_error) && attempt + 1 < MAX_ATTEMPTS {
                    backoff(attempt);
                    continue;
                }
                return Err(anyhow!(
                    "{context}: cozo sqlite backend panicked: {last_error}"
                ));
            }
        }
    }

    Err(anyhow!(
        "{context}: cozo sqlite backend stayed busy after {MAX_ATTEMPTS} attempts: {last_error}"
    ))
}

fn is_retryable_cozo_error(message: &str) -> bool {
    message.contains("database is locked") || message.contains("code: Some(5)")
}

fn backoff(attempt: usize) {
    let millis = 25 * (attempt as u64 + 1);
    thread::sleep(Duration::from_millis(millis));
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else {
        "unknown panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_errors_include_sqlite_busy_messages() {
        assert!(is_retryable_cozo_error("database is locked (code 5)"));
        assert!(is_retryable_cozo_error("Error { code: Some(5) }"));
        assert!(!is_retryable_cozo_error("relation not found"));
    }
}
