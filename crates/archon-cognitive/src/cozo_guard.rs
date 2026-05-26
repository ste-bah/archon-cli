use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

use crate::CognitiveError;

const MAX_ATTEMPTS: usize = 4;
static COZO_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn relation_count(
    db: &DbInstance,
    relation: &str,
    field: &str,
) -> Result<usize, CognitiveError> {
    let query = format!("?[{field}] := *{relation}{{{field}}}");
    let rows = run_script_guarded(
        db,
        query.as_str(),
        Default::default(),
        ScriptMutability::Immutable,
        "count cognitive relation",
    )?;
    Ok(rows.rows.len())
}

pub(crate) fn run_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows, CognitiveError> {
    let mut last_error = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        let guard = cozo_lock();
        let result = catch_unwind(AssertUnwindSafe(|| {
            db.run_script(script, params.clone(), mutability)
        }));
        drop(guard);
        match result {
            Ok(Ok(rows)) => return Ok(rows),
            Ok(Err(error)) => {
                last_error = error.to_string();
                if should_retry(&last_error, attempt) {
                    continue;
                }
                return Err(CognitiveError::Store(format!("{context}: {last_error}")));
            }
            Err(payload) => {
                last_error = panic_payload_message(payload.as_ref());
                if should_retry(&last_error, attempt) {
                    continue;
                }
                return Err(CognitiveError::Store(format!(
                    "{context}: cozo sqlite backend panicked: {last_error}"
                )));
            }
        }
    }
    Err(CognitiveError::Store(format!(
        "{context}: cozo sqlite backend stayed busy after {MAX_ATTEMPTS} attempts: {last_error}"
    )))
}

pub(crate) fn open_sqlite_guarded(
    path: &Path,
    context: &str,
) -> Result<DbInstance, CognitiveError> {
    let mut last_error = String::new();
    let path = path.to_string_lossy().to_string();
    for attempt in 0..MAX_ATTEMPTS {
        let guard = cozo_lock();
        let result = catch_unwind(AssertUnwindSafe(|| DbInstance::new("sqlite", &path, "")));
        drop(guard);
        match result {
            Ok(Ok(db)) => return Ok(db),
            Ok(Err(error)) => {
                last_error = error.to_string();
                if should_retry(&last_error, attempt) {
                    continue;
                }
                return Err(CognitiveError::Store(format!("{context}: {last_error}")));
            }
            Err(payload) => {
                last_error = panic_payload_message(payload.as_ref());
                if should_retry(&last_error, attempt) {
                    continue;
                }
                return Err(CognitiveError::Store(format!(
                    "{context}: cozo sqlite backend panicked: {last_error}"
                )));
            }
        }
    }
    Err(CognitiveError::Store(format!(
        "{context}: cozo sqlite backend stayed busy after {MAX_ATTEMPTS} attempts: {last_error}"
    )))
}

fn cozo_lock() -> std::sync::MutexGuard<'static, ()> {
    COZO_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn should_retry(message: &str, attempt: usize) -> bool {
    let retry = message.contains("database is locked") || message.contains("code: Some(5)");
    if retry && attempt + 1 < MAX_ATTEMPTS {
        thread::sleep(Duration::from_millis(25 * (attempt as u64 + 1)));
        true
    } else {
        false
    }
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
