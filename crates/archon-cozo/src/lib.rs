use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{Result, anyhow};
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

const DEFAULT_MAX_ATTEMPTS: usize = 90;
const INTERACTIVE_MAX_ATTEMPTS: usize = 10;
const DEFAULT_INITIAL_BACKOFF_MS: u64 = 100;
const DEFAULT_MAX_BACKOFF_MS: u64 = 2_000;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum WriteLockKey {
    Fallback,
    Path(PathBuf),
}

static COZO_PROCESS_WRITE_LOCKS: OnceLock<Mutex<HashMap<WriteLockKey, Arc<Mutex<()>>>>> =
    OnceLock::new();
static COZO_PANIC_HOOK: OnceLock<()> = OnceLock::new();

thread_local! {
    static IN_GUARDED_COZO_OPERATION: Cell<usize> = const { Cell::new(0) };
}

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

pub async fn open_sqlite_guarded_async(
    path: &str,
    context: &str,
    config: &CozoGuardConfig,
) -> Result<DbInstance> {
    run_guarded_async(context, ScriptMutability::Mutable, config, || {
        DbInstance::new("sqlite", path, "")
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
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
    config: &CozoGuardConfig,
) -> Result<NamedRows> {
    run_guarded_async(context, mutability, config, || {
        db.run_script(script, params.clone(), mutability)
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
    let attempts = config.max_attempts.max(1);

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

pub async fn run_guarded_async<T>(
    context: &str,
    mutability: ScriptMutability,
    config: &CozoGuardConfig,
    mut run: impl FnMut() -> Result<T>,
) -> Result<T> {
    let attempts = config.max_attempts.max(1);

    for attempt in 0..attempts {
        match run_guarded_once(context, mutability, config, &mut run) {
            Ok(value) => return Ok(value),
            Err(error) => {
                let last_error = format!("{error:#}");
                if let Some(backoff) =
                    retry_backoff(context, config, attempt, attempts, &last_error)
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
    [
        "database is locked",
        "database table is locked",
        "locked (code 5)",
        "code: some(5)",
        "sqlite_busy",
        "poisonerror",
        "poison error",
        "wouldblock",
        "would-block",
        "would block",
        "write-lock unavailable",
        "write lock unavailable",
    ]
    .iter()
    .any(|signal| message.contains(signal))
}

fn run_guarded_once<T>(
    context: &str,
    mutability: ScriptMutability,
    config: &CozoGuardConfig,
    run: &mut impl FnMut() -> Result<T>,
) -> Result<T> {
    if matches!(mutability, ScriptMutability::Mutable) {
        let process_lock = process_write_lock(config.write_lock_path.as_deref())?;
        let _process_guard = lock_recovering_poison(&process_lock);
        if let Some(path) = &config.write_lock_path {
            return with_write_lock(path, context, || catch_guarded_operation(context, run));
        }
        return catch_guarded_operation(context, run);
    }

    catch_guarded_operation(context, run)
}

fn process_write_lock(path: Option<&Path>) -> Result<Arc<Mutex<()>>> {
    let key = match path {
        Some(path) => WriteLockKey::Path(normalized_lock_path(path)?),
        None => WriteLockKey::Fallback,
    };
    let locks = COZO_PROCESS_WRITE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = lock_recovering_poison(locks);
    Ok(Arc::clone(
        locks.entry(key).or_insert_with(|| Arc::new(Mutex::new(()))),
    ))
}

fn normalized_lock_path(path: &Path) -> Result<PathBuf> {
    canonical_resource_path(path)
}

/// Return a stable path key for a possibly absent resource.
///
/// The parent is created and canonicalized so relative paths, `.`/`..`, and
/// symlinked parents resolve to one process-wide resource identity.
pub fn canonical_resource_path(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = absolute_normalized_path(path.as_ref())?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let parent = canonicalize_existing_path(parent)?;
    Ok(parent.join(path.file_name().unwrap_or_default()))
}

fn absolute_normalized_path(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(lexically_normalized_path(&path))
}

fn canonicalize_existing_path(path: &Path) -> Result<PathBuf> {
    let mut unresolved = Vec::new();
    let mut existing = path;

    loop {
        match existing.canonicalize() {
            Ok(mut normalized) => {
                for component in unresolved.iter().rev() {
                    normalized.push(component);
                }
                return Ok(normalized);
            }
            Err(error) => {
                let Some(name) = existing.file_name() else {
                    return Err(error.into());
                };
                unresolved.push(name.to_os_string());
                let Some(parent) = existing.parent() else {
                    return Err(error.into());
                };
                existing = parent;
            }
        }
    }
}

fn lexically_normalized_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn lock_recovering_poison<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
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
    install_cozo_panic_hook();
    let _guard = GuardedCozoOperation::enter();
    let result = catch_unwind(AssertUnwindSafe(run));

    match result {
        Ok(result) => result,
        Err(payload) => Err(anyhow!(
            "{context}: Cozo operation panicked: {}",
            panic_payload_message(payload)
        )),
    }
}

fn install_cozo_panic_hook() {
    COZO_PANIC_HOOK.get_or_init(|| {
        let delegate = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let guarded = IN_GUARDED_COZO_OPERATION.with(|depth| depth.get() > 0);
            if !guarded {
                delegate(panic_info);
            }
        }));
    });
}

struct GuardedCozoOperation;

impl GuardedCozoOperation {
    fn enter() -> Self {
        IN_GUARDED_COZO_OPERATION.with(|depth| depth.set(depth.get() + 1));
        Self
    }
}

impl Drop for GuardedCozoOperation {
    fn drop(&mut self) {
        IN_GUARDED_COZO_OPERATION.with(|depth| depth.set(depth.get().saturating_sub(1)));
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
mod tests;
