use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

static TARGET_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();

pub(crate) struct CargoTargetDirLock {
    _guard: OwnedMutexGuard<()>,
}

pub(crate) async fn apply_cargo_target_dir_guard(
    env: &mut Vec<(String, String)>,
    command: &str,
    working_dir: &Path,
    cancel: Option<CancellationToken>,
) -> Result<Option<CargoTargetDirLock>, String> {
    let Some(target_dir) =
        guarded_cargo_target_dir(command, working_dir, env_has_cargo_target(env))
    else {
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
    env.push((
        "CARGO_TARGET_DIR".to_string(),
        target_dir.display().to_string(),
    ));
    Ok(Some(lock_target_dir(target_dir, cancel).await?))
}

fn guarded_cargo_target_dir(
    command: &str,
    working_dir: &Path,
    env_already_has_target: bool,
) -> Option<PathBuf> {
    if env_already_has_target
        || command.contains("CARGO_TARGET_DIR")
        || !contains_shell_word(command, "cargo")
        || !is_macos_external_volume(working_dir)
    {
        return None;
    }
    Some(local_target_root().join(stable_path_hash(&canonical_working_dir(working_dir))))
}

fn env_has_cargo_target(env: &[(String, String)]) -> bool {
    env.iter().any(|(key, _)| key == "CARGO_TARGET_DIR")
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

fn local_target_root() -> PathBuf {
    let temp = std::env::temp_dir();
    let root = if temp.starts_with("/Volumes/") {
        PathBuf::from("/tmp")
    } else {
        temp
    };
    root.join("archon-cargo-target")
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
    Ok(CargoTargetDirLock { _guard: guard })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_word_detection_matches_cargo_not_substrings() {
        assert!(contains_shell_word("cargo test -p demo", "cargo"));
        assert!(contains_shell_word(
            "cd repo && cargo +nightly test",
            "cargo"
        ));
        assert!(!contains_shell_word("echo cargo-test", "cargo"));
        assert!(!contains_shell_word("echo xcargo", "cargo"));
    }

    #[test]
    fn stable_hash_is_deterministic() {
        let path = Path::new("/Volumes/Externalwork/demo");
        assert_eq!(stable_path_hash(path), stable_path_hash(path));
        assert_ne!(
            stable_path_hash(path),
            stable_path_hash(Path::new("/Volumes/Externalwork/other"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn external_volume_cargo_gets_local_target_dir() {
        let target =
            guarded_cargo_target_dir("cargo test", Path::new("/Volumes/Externalwork/demo"), false)
                .expect("external Cargo command should be guarded");
        assert!(target.starts_with(local_target_root()));
    }

    #[test]
    fn explicit_cargo_target_dir_is_preserved() {
        assert!(
            guarded_cargo_target_dir(
                "CARGO_TARGET_DIR=/tmp/target cargo test",
                Path::new("/Volumes/Externalwork/demo"),
                false,
            )
            .is_none()
        );
        assert!(
            guarded_cargo_target_dir("cargo test", Path::new("/Volumes/Externalwork/demo"), true)
                .is_none()
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn cargo_target_lock_serializes_same_external_repo() {
        let mut first_env = Vec::new();
        let first = apply_cargo_target_dir_guard(
            &mut first_env,
            "cargo test",
            Path::new("/Volumes/Externalwork/demo"),
            None,
        )
        .await
        .unwrap()
        .expect("first cargo command should lock target");

        let mut second_env = Vec::new();
        let second = apply_cargo_target_dir_guard(
            &mut second_env,
            "cargo test",
            Path::new("/Volumes/Externalwork/demo"),
            None,
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), second)
                .await
                .is_err(),
            "second cargo command should wait for the same target lock"
        );
        drop(first);
    }
}
