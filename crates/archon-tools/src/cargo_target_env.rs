use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

static TARGET_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
static EXEMPT_WAITS: OnceLock<std::sync::Mutex<HashMap<String, CargoWaitState>>> = OnceLock::new();

#[derive(Default)]
struct CargoWaitState {
    completed: Duration,
    active_since: Option<Instant>,
    active_count: usize,
}

struct CargoWaitRegistration {
    session_id: String,
    active: bool,
}

pub(crate) struct CargoTargetDirLock {
    _guard: OwnedMutexGuard<()>,
    target_dir: PathBuf,
    repair_incomplete_tree_sitter: bool,
}

pub(crate) async fn apply_cargo_target_dir_guard(
    env: &mut Vec<(String, String)>,
    command: &str,
    working_dir: &Path,
    session_id: &str,
    cancel: Option<CancellationToken>,
) -> Result<Option<CargoTargetDirLock>, String> {
    let Some(target_dir) = guarded_cargo_target_dir(command, working_dir) else {
        return Ok(None);
    };
    if let Err(error) = std::fs::create_dir_all(&target_dir) {
        tracing::warn!(
            path = %target_dir.display(),
            %error,
            "bash: failed to create local Cargo target dir guard"
        );
        return Ok(None);
    }
    env.retain(|(key, _)| key != "CARGO_TARGET_DIR" && key != "ARCHON_CARGO_TARGET_DIR");
    let target_dir_value = target_dir.display().to_string();
    env.push(("CARGO_TARGET_DIR".to_string(), target_dir_value.clone()));
    env.push(("ARCHON_CARGO_TARGET_DIR".to_string(), target_dir_value));
    let mut wait = CargoWaitRegistration::begin(session_id);
    let mut lock = lock_target_dir(target_dir, cancel).await?;
    lock.repair_incomplete_tree_sitter = incomplete_tree_sitter_build_output(&lock.target_dir);
    wait.finish();
    Ok(Some(lock))
}

pub fn current_timeout_exempt_cargo_wait(session_id: &str) -> Duration {
    let waits = EXEMPT_WAITS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    waits
        .lock()
        .ok()
        .and_then(|waits| waits.get(session_id).map(CargoWaitState::total))
        .unwrap_or_default()
}

pub fn take_timeout_exempt_cargo_wait(session_id: &str) -> Duration {
    if session_id.is_empty() {
        return Duration::ZERO;
    }
    let waits = EXEMPT_WAITS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    waits
        .lock()
        .ok()
        .and_then(|mut waits| waits.remove(session_id).map(|state| state.total()))
        .unwrap_or_default()
}

impl CargoWaitState {
    fn total(&self) -> Duration {
        let active = self
            .active_since
            .map(|started| started.elapsed())
            .unwrap_or_default();
        self.completed + active
    }
}

impl CargoWaitRegistration {
    fn begin(session_id: &str) -> Self {
        if !session_id.is_empty() {
            update_wait_state(session_id, true);
        }
        Self {
            session_id: session_id.to_string(),
            active: !session_id.is_empty(),
        }
    }

    fn finish(&mut self) {
        if self.active {
            update_wait_state(&self.session_id, false);
            self.active = false;
        }
    }
}

impl Drop for CargoWaitRegistration {
    fn drop(&mut self) {
        self.finish();
    }
}

fn update_wait_state(session_id: &str, starting: bool) {
    let waits = EXEMPT_WAITS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let Ok(mut waits) = waits.lock() else {
        return;
    };
    let state = waits.entry(session_id.to_string()).or_default();
    if starting {
        if state.active_count == 0 {
            state.active_since = Some(Instant::now());
        }
        state.active_count += 1;
    } else if state.active_count > 0 {
        state.active_count -= 1;
        if state.active_count == 0
            && let Some(started) = state.active_since.take()
        {
            state.completed += started.elapsed();
        }
    }
}

fn guarded_cargo_target_dir(command: &str, working_dir: &Path) -> Option<PathBuf> {
    if !contains_shell_word(command, "cargo") || !is_macos_external_volume(working_dir) {
        return None;
    }
    Some(local_target_root().join(stable_path_hash(&repository_identity(working_dir))))
}

pub(crate) fn enforce_host_cargo_target_dir(command: &str, guarded: bool) -> String {
    if !guarded || !command.contains("CARGO_TARGET_DIR") {
        return command.to_string();
    }
    let assignment =
        regex::Regex::new(r#"\bCARGO_TARGET_DIR=(?:\"[^\"\n]*\"|'[^'\n]*'|[^\s;&|)]+)"#)
            .expect("valid Cargo target assignment regex");
    assignment
        .replace_all(command, r#"CARGO_TARGET_DIR="$${ARCHON_CARGO_TARGET_DIR}""#)
        .into_owned()
}

/// Shell run before the caller's command when the cache needs repairing.
///
/// Invisible when it works, loud when it does not, and neither of those was
/// true before.
///
/// It used to be `cargo clean -p tree-sitter >/dev/null || exit $?`, which
/// redirects stdout — and cargo writes `Removed 15 files, 50.2KiB total` to
/// *stderr*. Archon captures stderr into the tool result, so the line survived
/// and was prepended to whatever the caller's command printed. Agents read that
/// output: a repair that silently prefixes a line corrupts the answer, and only
/// when the cache happens to be dirty, which is about the worst debugging
/// experience a bug can offer.
///
/// `2>&1` alone would fix the corruption and introduce a quieter fault in its
/// place: with stderr discarded, a repair that *fails* aborts the whole tool
/// call through `exit $?` with a status and no reason. So capture stderr rather
/// than discard it, drop it on success, and print it on failure — the caller's
/// command still does not run, but now there is something to read about why.
pub(crate) fn cargo_cache_repair_prelude(lock: Option<&CargoTargetDirLock>) -> &'static str {
    cargo_cache_repair_prelude_for(lock.is_some_and(|lock| lock.repair_incomplete_tree_sitter))
}

/// The shell itself, split from the lock lookup so a test can read it.
///
/// `CargoTargetDirLock` owns a mutex guard, so asserting on the prelude meant
/// fabricating one -- which is why nothing ever did, and why a stderr leak sat
/// in a string no test could see.
fn cargo_cache_repair_prelude_for(repair: bool) -> &'static str {
    if repair {
        // `2>&1 >/dev/null` in that order: stderr goes to the capture, stdout to
        // the bin. Reversing it would capture stdout and leak stderr, which is
        // the bug this replaces.
        r#"__archon_clean=$(cargo clean -p tree-sitter 2>&1 >/dev/null); __archon_rc=$?
[ "$__archon_rc" -eq 0 ] || { printf '%s\n' "$__archon_clean" >&2; exit "$__archon_rc"; }"#
    } else {
        ""
    }
}

fn incomplete_tree_sitter_build_output(target_dir: &Path) -> bool {
    ["debug", "release"].into_iter().any(|profile| {
        let build_root = target_dir.join(profile).join("build");
        std::fs::read_dir(build_root)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("tree-sitter-"))
            })
            .any(|path| {
                let out = path.join("out");
                path.join("invoked.timestamp").is_file()
                    && out.join("libtree-sitter.a").is_file()
                    && !out.join("stdlib-symbols.txt").is_file()
            })
    })
}

fn contains_shell_word(command: &str, needle: &str) -> bool {
    command.match_indices(needle).any(|(idx, _)| {
        let before = command[..idx].chars().next_back();
        let after = command[idx + needle.len()..].chars().next();
        !is_word_char(before) && !is_word_char(after)
    })
}

fn is_word_char(ch: Option<char>) -> bool {
    ch.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn is_macos_external_volume(working_dir: &Path) -> bool {
    cfg!(target_os = "macos") && canonical_working_dir(working_dir).starts_with("/Volumes/")
}

fn canonical_working_dir(working_dir: &Path) -> PathBuf {
    std::fs::canonicalize(working_dir).unwrap_or_else(|_| working_dir.to_path_buf())
}

fn repository_identity(working_dir: &Path) -> PathBuf {
    let canonical = canonical_working_dir(working_dir);
    for ancestor in canonical.ancestors() {
        let marker = ancestor.join(".git");
        if marker.is_dir() {
            return std::fs::canonicalize(ancestor).unwrap_or_else(|_| ancestor.to_path_buf());
        }
        if let Some(git_dir) = git_dir_from_file(&marker, ancestor) {
            return repository_root_from_git_dir(&git_dir);
        }
    }
    canonical
}

fn git_dir_from_file(marker: &Path, repo_root: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(marker).ok()?;
    let path = raw.trim().strip_prefix("gitdir:")?.trim();
    let git_dir = PathBuf::from(path);
    let resolved = if git_dir.is_absolute() {
        git_dir
    } else {
        repo_root.join(git_dir)
    };
    Some(std::fs::canonicalize(&resolved).unwrap_or(resolved))
}

fn common_git_dir(git_dir: &Path) -> PathBuf {
    let Some(worktrees) = git_dir.parent() else {
        return git_dir.to_path_buf();
    };
    if worktrees.file_name().and_then(|name| name.to_str()) != Some("worktrees") {
        return git_dir.to_path_buf();
    }
    worktrees
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| git_dir.to_path_buf())
}

fn repository_root_from_git_dir(git_dir: &Path) -> PathBuf {
    let common = common_git_dir(git_dir);
    common.parent().map(Path::to_path_buf).unwrap_or(common)
}

fn local_target_root() -> PathBuf {
    let temp = std::env::temp_dir();
    local_target_root_for_temp(&temp)
}

fn local_target_root_for_temp(temp: &Path) -> PathBuf {
    temp.join("archon-cargo-target")
}

fn stable_path_hash(path: &Path) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

async fn lock_target_dir(
    target_dir: PathBuf,
    cancel: Option<CancellationToken>,
) -> Result<CargoTargetDirLock, String> {
    let lock = {
        let locks = TARGET_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut locks = locks.lock().await;
        locks
            .entry(target_dir.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let guard = match cancel {
        Some(cancel) => {
            tokio::select! {
                guard = lock.lock_owned() => guard,
                _ = cancel.cancelled() => {
                    return Err(format!(
                        "cancelled while waiting for Cargo target dir lock: {}",
                        target_dir.display()
                    ));
                }
            }
        }
        None => lock.lock_owned().await,
    };
    Ok(CargoTargetDirLock {
        _guard: guard,
        target_dir,
        repair_incomplete_tree_sitter: false,
    })
}

#[cfg(test)]
#[path = "cargo_target_env_tests.rs"]
mod tests;
