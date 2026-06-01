use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

const DEFAULT_MAX_ATTEMPTS: usize = 90;
const DEFAULT_INITIAL_BACKOFF_MS: u64 = 100;
const DEFAULT_MAX_BACKOFF_MS: u64 = 2_000;

static COZO_PROCESS_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static COZO_PANIC_HOOK_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Debug)]
pub struct CozoGuardConfig {
    pub max_attempts: usize,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub write_lock_path: Option<PathBuf>,
}

impl Default for CozoGuardConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            initial_backoff: Duration::from_millis(DEFAULT_INITIAL_BACKOFF_MS),
            max_backoff: Duration::from_millis(DEFAULT_MAX_BACKOFF_MS),
            write_lock_path: None,
        }
    }
}

impl CozoGuardConfig {
    pub fn for_db_path(path: impl AsRef<Path>) -> Self {
        Self::default().with_write_lock_path(write_lock_path_for_db(path))
    }

    pub fn with_write_lock_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.write_lock_path = Some(path.into());
        self
    }
}

pub fn write_lock_path_for_db(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("cozo.db");
    path.with_file_name(format!("{file_name}.archon-cozo-write.lock"))
}

pub fn open_sqlite_guarded(
    path: &str,
    context: &str,
    config: &CozoGuardConfig,
) -> Result<DbInstance> {
    run_guarded(context, ScriptMutability::Mutable, config, || {
        DbInstance::new("sqlite", path, "")
            .map_err(|error| anyhow!("open sqlite-backed Cozo store failed: {error}"))
    })
}

pub fn run_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
    config: &CozoGuardConfig,
) -> Result<NamedRows> {
    run_guarded(context, mutability, config, || {
        db.run_script(script, params.clone(), mutability)
            .map_err(|error| anyhow!("{error}"))
    })
}

pub fn run_guarded<T>(
    context: &str,
    mutability: ScriptMutability,
    config: &CozoGuardConfig,
    mut run: impl FnMut() -> Result<T>,
) -> Result<T> {
    let attempts = config.max_attempts.max(1);
    let mut last_error = String::new();

    for attempt in 0..attempts {
        let result = run_guarded_once(context, mutability, config, &mut run);
        match result {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = format!("{error:#}");
                if is_retryable_cozo_error(&last_error) && attempt + 1 < attempts {
                    tracing::warn!(
                        context,
                        attempt = attempt + 1,
                        max_attempts = attempts,
                        error = %last_error,
                        "Cozo store busy; retrying guarded operation"
                    );
                    thread::sleep(backoff_duration(config, attempt));
                    continue;
                }
                return Err(anyhow!("{context}: {last_error}"));
            }
        }
    }

    Err(anyhow!(
        "{context}: Cozo store stayed busy after {attempts} attempts: {last_error}"
    ))
}

pub fn is_retryable_cozo_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("database is locked")
        || message.contains("database table is locked")
        || message.contains("locked (code 5)")
        || message.contains("code 5")
        || message.contains("code: some(5)")
        || message.contains("sqlite_busy")
        || message.contains("poisonerror")
        || message.contains("wouldblock")
        || message.contains("would block")
        || message.contains("write lock unavailable")
}

fn run_guarded_once<T>(
    context: &str,
    mutability: ScriptMutability,
    config: &CozoGuardConfig,
    run: &mut impl FnMut() -> Result<T>,
) -> Result<T> {
    if matches!(mutability, ScriptMutability::Mutable) {
        let process_lock = COZO_PROCESS_WRITE_LOCK.get_or_init(|| Mutex::new(()));
        let _process_guard = match process_lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(path) = &config.write_lock_path {
            return with_write_lock(path, context, || catch_guarded_operation(context, run));
        }
    }

    catch_guarded_operation(context, run)
}

pub fn with_write_lock<T>(
    path: &Path,
    context: &str,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            anyhow!(
                "{context}: open Cozo write lock {}: {error}",
                path.display()
            )
        })?;
    let mut lock = fd_lock::RwLock::new(file);
    let _guard = lock.try_write().map_err(|error| {
        anyhow!(
            "{context}: Cozo write lock unavailable at {}: {error}",
            path.display()
        )
    })?;
    tracing::trace!(context, lock_path = %path.display(), "acquired Cozo write lock");
    run()
}

fn catch_guarded_operation<T>(context: &str, run: &mut impl FnMut() -> Result<T>) -> Result<T> {
    let hook_lock = COZO_PANIC_HOOK_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = match hook_lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = catch_unwind(AssertUnwindSafe(run));
    std::panic::set_hook(hook);

    match result {
        Ok(result) => result,
        Err(payload) => Err(anyhow!(
            "{context}: Cozo operation panicked: {}",
            panic_payload_message(payload)
        )),
    }
}

fn backoff_duration(config: &CozoGuardConfig, attempt: usize) -> Duration {
    let initial = config.initial_backoff.as_millis() as u64;
    let max = config.max_backoff.as_millis() as u64;
    Duration::from_millis(initial.saturating_mul(attempt as u64 + 1).min(max))
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        "unknown panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_lock_path_is_sibling_sidecar() {
        let path = PathBuf::from("/tmp/archon-data.db");
        assert_eq!(
            write_lock_path_for_db(&path),
            PathBuf::from("/tmp/archon-data.db.archon-cozo-write.lock")
        );
    }

    #[test]
    fn retryable_errors_include_sqlite_and_file_lock_variants() {
        assert!(is_retryable_cozo_error("database is locked (code 5)"));
        assert!(is_retryable_cozo_error("sqlite_busy"));
        assert!(is_retryable_cozo_error("Cozo write lock unavailable"));
        assert!(!is_retryable_cozo_error("relation not found"));
    }

    #[test]
    fn write_lock_rejects_second_writer() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        let mut lock = fd_lock::RwLock::new(file);
        let _guard = lock.try_write().unwrap();

        let error = with_write_lock(&path, "test lock", || Ok(())).unwrap_err();

        assert!(is_retryable_cozo_error(&format!("{error:#}")));
    }
}
