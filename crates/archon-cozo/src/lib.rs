use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

mod guard_registry;
mod locking;
mod panic_guard;

use guard_registry::register_guarded_database;
use locking::{
    HeldWriteLock, lock_recovering_poison, process_write_lock, write_lock_is_held, write_lock_key,
};
use panic_guard::catch_guarded_operation;

/// How long [`with_write_lock_blocking`] waits before declaring the holder stuck.
///
/// Sized for the worst realistic queue rather than the common case. Every
/// guarded mutable Cozo operation in the workspace funnels through this lock,
/// several of them now holding it across a whole `multi_transaction` rather
/// than a single `:put`, and the fail-fast path already spends up to 19s of
/// cumulative backoff before it gives up (`cumulative_backoff_budget`). A
/// ceiling at or below that would report a timeout while the system is merely
/// busy. This exists to turn a wedged or leaked lock into a diagnosable error,
/// not to police contention.
pub const DEFAULT_WRITE_LOCK_WAIT: Duration = Duration::from_secs(60);

const DEFAULT_MAX_ATTEMPTS: usize = 20;
const INTERACTIVE_MAX_ATTEMPTS: usize = 10;
const DEFAULT_INITIAL_BACKOFF_MS: u64 = 100;
const DEFAULT_MAX_BACKOFF_MS: u64 = 2_000;

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

    pub fn for_interactive_db_path(path: impl AsRef<Path>) -> Self {
        Self {
            max_attempts: INTERACTIVE_MAX_ATTEMPTS,
            ..Self::for_db_path(path)
        }
    }

    pub fn with_write_lock_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.write_lock_path = Some(path.into());
        self
    }
}

#[derive(Clone)]
pub struct GuardedDbInstance {
    db: Arc<DbInstance>,
    config: CozoGuardConfig,
}

impl GuardedDbInstance {
    pub fn new(db: DbInstance, config: CozoGuardConfig) -> Self {
        let db = Arc::new(db);
        register_guarded_database(&db, &config);
        Self { db, config }
    }

    pub fn db(&self) -> &DbInstance {
        &self.db
    }

    pub fn db_arc(&self) -> Arc<DbInstance> {
        Arc::clone(&self.db)
    }

    pub fn config(&self) -> &CozoGuardConfig {
        &self.config
    }

    pub fn run_script_guarded(
        &self,
        script: &str,
        params: BTreeMap<String, DataValue>,
        mutability: ScriptMutability,
        context: &str,
    ) -> Result<NamedRows> {
        run_script_guarded(&self.db, script, params, mutability, context, &self.config)
    }
}

impl std::ops::Deref for GuardedDbInstance {
    type Target = DbInstance;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

pub fn guarded_config_for(db: &DbInstance) -> Option<CozoGuardConfig> {
    guard_registry::guarded_config_for(db)
}

pub fn bound_guard_config(db: &DbInstance, context: &str) -> Result<CozoGuardConfig> {
    guard_registry::bound_guard_config(db, context)
}

pub fn run_bound_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows> {
    let config = bound_guard_config(db, context)?;
    run_script_guarded(db, script, params, mutability, context, &config)
}

pub fn run_bound_guarded<T>(
    db: &DbInstance,
    context: &str,
    mutability: ScriptMutability,
    run: impl FnMut() -> Result<T>,
) -> Result<T> {
    let config = bound_guard_config(db, context)?;
    run_guarded(context, mutability, &config, run)
}

pub fn write_lock_path_for_db(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let resource = canonical_resource_path(path).unwrap_or_else(|_| path.to_path_buf());
    let file_name = resource
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("cozo.db");
    resource.with_file_name(format!("{file_name}.archon-cozo-write.lock"))
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

pub fn open_sqlite_guarded_instance(
    path: &str,
    context: &str,
    config: CozoGuardConfig,
) -> Result<GuardedDbInstance> {
    let db = open_sqlite_guarded(path, context, &config)?;
    Ok(GuardedDbInstance::new(db, config))
}

pub async fn open_sqlite_guarded_async(
    path: &str,
    context: &str,
    config: &CozoGuardConfig,
) -> Result<DbInstance> {
    let path = path.to_string();
    run_guarded_async(context, ScriptMutability::Mutable, config, move || {
        DbInstance::new("sqlite", &path, "")
            .map_err(|error| anyhow!("open sqlite-backed Cozo store failed: {error}"))
    })
    .await
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

pub async fn run_script_guarded_async(
    db: Arc<DbInstance>,
    script: impl Into<String>,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
    config: &CozoGuardConfig,
) -> Result<NamedRows> {
    let script = script.into();
    run_guarded_async(context, mutability, config, move || {
        db.run_script(&script, params.clone(), mutability)
            .map_err(|error| anyhow!("{error}"))
    })
    .await
}

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
                if let Some(backoff) =
                    retry_backoff(context, config, attempt, attempts, &last_error)
                {
                    thread::sleep(backoff);
                    continue;
                }
                return Err(anyhow!("{context}: {last_error}"));
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
                if let Some(backoff) =
                    retry_backoff(&context, &config, attempt, attempts, &last_error)
                {
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                return Err(anyhow!("{context}: {last_error}"));
            }
        }
    }

    unreachable!("a guarded retry loop always returns from an attempt")
}

fn normalized_attempts(config: &CozoGuardConfig) -> usize {
    config.max_attempts.max(1)
}

#[cfg(test)]
fn cumulative_backoff_budget(config: &CozoGuardConfig) -> Duration {
    (0..normalized_attempts(config).saturating_sub(1))
        .map(|attempt| backoff_duration(config, attempt))
        .sum()
}

fn retry_backoff(
    context: &str,
    config: &CozoGuardConfig,
    attempt: usize,
    attempts: usize,
    error: &str,
) -> Option<Duration> {
    if !is_retryable_cozo_error(error) || attempt + 1 >= attempts {
        return None;
    }

    tracing::warn!(
        context,
        attempt = attempt + 1,
        max_attempts = attempts,
        error,
        "Cozo store busy; retrying guarded operation"
    );
    Some(backoff_duration(config, attempt))
}

pub fn is_retryable_cozo_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    if explicit_error_codes(&message).any(|code| code != 5) {
        return false;
    }
    [
        "database is locked",
        "database table is locked",
        "locked (code 5)",
        "code: some(5)",
        "sqlite_busy",
        "write-lock unavailable",
        "write lock unavailable",
    ]
    .iter()
    .any(|signal| message.contains(signal))
}

fn explicit_error_codes(message: &str) -> impl Iterator<Item = u64> + '_ {
    message.match_indices("code").filter_map(|(index, _)| {
        let suffix = &message[index + "code".len()..];
        let suffix = suffix.trim_start_matches(|character: char| {
            character.is_ascii_whitespace() || matches!(character, ':' | '(')
        });
        let suffix = suffix.strip_prefix("some(").unwrap_or(suffix);
        let digits = suffix
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        digits.parse().ok()
    })
}

fn run_guarded_once<T>(
    context: &str,
    mutability: ScriptMutability,
    config: &CozoGuardConfig,
    run: &mut impl FnMut() -> Result<T>,
) -> Result<T> {
    if matches!(mutability, ScriptMutability::Mutable) {
        let key = write_lock_key(config.write_lock_path.as_deref())?;
        if write_lock_is_held(&key) {
            return catch_guarded_operation(context, run);
        }
        let process_lock = process_write_lock(&key);
        let _process_guard = lock_recovering_poison(&process_lock);
        let _held_lock = HeldWriteLock::enter(key);
        if let Some(path) = &config.write_lock_path {
            return with_write_lock(path, context, || catch_guarded_operation(context, run));
        }
        return catch_guarded_operation(context, run);
    }

    catch_guarded_operation(context, run)
}

pub fn canonical_resource_path(path: impl AsRef<Path>) -> Result<PathBuf> {
    locking::canonical_resource_path(path)
}

/// Fail fast if the write lock for `path` is already taken.
///
/// The caller is expected to treat the error as retryable and come back with
/// backoff; `run_guarded_once` does exactly that. Use
/// [`with_write_lock_blocking`] instead when losing the race is not something
/// the caller can recover from by retrying a whole operation.
pub fn with_write_lock<T>(
    path: &Path,
    context: &str,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    locking::with_write_lock(path, context, run)
}

/// Run `run` under the write lock for `path`, waiting up to
/// [`DEFAULT_WRITE_LOCK_WAIT`] for a current holder.
///
/// Re-entrant: a thread that already holds this lock — including one inside a
/// guarded mutable operation on the same database — runs `run` inline instead
/// of deadlocking against its own `LockFileEx` byte-range lock.
pub fn with_write_lock_blocking<T>(
    path: &Path,
    context: &str,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    with_write_lock_blocking_timeout(path, context, DEFAULT_WRITE_LOCK_WAIT, run)
}

/// [`with_write_lock_blocking`] with an explicit ceiling on the acquire.
///
/// `wait` bounds only the acquisition. Once the lock is held, `run` is allowed
/// to take as long as it needs.
pub fn with_write_lock_blocking_timeout<T>(
    path: &Path,
    context: &str,
    wait: Duration,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    locking::with_write_lock_blocking(path, context, wait, run)
}

pub fn in_guarded_operation() -> bool {
    panic_guard::in_guarded_operation()
}

fn backoff_duration(config: &CozoGuardConfig, attempt: usize) -> Duration {
    let initial = config.initial_backoff.as_millis() as u64;
    let max = config.max_backoff.as_millis() as u64;
    Duration::from_millis(initial.saturating_mul(attempt as u64 + 1).min(max))
}

#[cfg(test)]
mod storage_evidence_tests;
#[cfg(test)]
mod tests;
