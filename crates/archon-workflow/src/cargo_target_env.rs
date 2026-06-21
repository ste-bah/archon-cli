use std::path::{Path, PathBuf};

pub(crate) fn guarded_cargo_target_env(
    command: &str,
    working_dir: &Path,
) -> Option<(String, String)> {
    let target_dir = guarded_cargo_target_dir(
        command,
        working_dir,
        std::env::var_os("CARGO_TARGET_DIR").is_some(),
    )?;
    if std::fs::create_dir_all(&target_dir).is_err() {
        return None;
    }
    Some((
        "CARGO_TARGET_DIR".to_string(),
        target_dir.display().to_string(),
    ))
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
        let target = guarded_cargo_target_dir("cargo test", Path::new("/Volumes/demo"), false)
            .expect("external Cargo command should be guarded");
        assert!(target.starts_with(local_target_root()));
    }

    #[test]
    fn explicit_cargo_target_dir_is_preserved() {
        assert!(
            guarded_cargo_target_dir(
                "CARGO_TARGET_DIR=/tmp/target cargo test",
                Path::new("/Volumes/demo"),
                false,
            )
            .is_none()
        );
        assert!(guarded_cargo_target_dir("cargo test", Path::new("/Volumes/demo"), true).is_none());
    }
}
