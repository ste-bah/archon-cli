//! Operator-selected storage roots shared by cache leases and shell preparation.
use std::{
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

#[derive(Clone, Default)]
struct Roots {
    cache: Option<PathBuf>,
    scratch: Option<PathBuf>,
}
static ROOTS: OnceLock<RwLock<Roots>> = OnceLock::new();

pub fn configure(cache: Option<PathBuf>, scratch: Option<PathBuf>) -> Result<(), String> {
    for (name, path) in [
        ("tools.cache_root", &cache),
        ("tools.scratch_root", &scratch),
    ] {
        if let Some(path) = path {
            if !path.is_absolute() {
                return Err(format!("{name} must be an absolute path"));
            }
            std::fs::create_dir_all(path).map_err(|e| format!("{name}: {e}"))?;
            if !path.is_dir() {
                return Err(format!("{name} must name a directory"));
            }
        }
    }
    *ROOTS
        .get_or_init(Default::default)
        .write()
        .map_err(|_| "cache root lock poisoned")? = Roots { cache, scratch };
    Ok(())
}

pub fn cache_root() -> Option<PathBuf> {
    std::env::var_os("ARCHON_CACHE_ROOT")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| ROOTS.get().and_then(|r| r.read().ok()?.cache.clone()))
}
pub fn scratch_root() -> PathBuf {
    std::env::var_os("ARCHON_TMPDIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| ROOTS.get().and_then(|r| r.read().ok()?.scratch.clone()))
        .unwrap_or_else(std::env::temp_dir)
}
pub fn pool_root(default: PathBuf) -> PathBuf {
    cache_root()
        .map(|root| root.join("build-cache"))
        .unwrap_or(default)
}
pub fn cargo_fallback_root() -> PathBuf {
    cache_root()
        .unwrap_or_else(scratch_root)
        .join("archon-cargo-target")
}

/// Toolchain discovery, not shell command spelling, decides cache variables.
///
/// Returns proof that the unleased cache entry is in use, for the caller to
/// hold until the command finishes. Dropping it early would let a concurrent
/// sweep consider the entry idle while a build is still writing into it.
pub fn apply_shell_roots(
    env: &mut Vec<(String, String)>,
    repository: &Path,
    extra: &[String],
    leased: bool,
) -> Result<Option<crate::cache_gc::CacheEntryGuard>, String> {
    let scratch = scratch_root();
    if !scratch.is_absolute() {
        return Err("ARCHON_TMPDIR/tools.scratch_root must be absolute".into());
    }
    std::fs::create_dir_all(&scratch).map_err(|e| format!("scratch root: {e}"))?;
    for key in ["TMPDIR", "TMP", "TEMP"] {
        crate::bash::bash_env::set_env_override(env, key, &scratch.to_string_lossy());
    }
    let configured = cache_root();
    let external = cfg!(target_os = "macos") && repository.starts_with("/Volumes/");
    if leased || (configured.is_none() && !external) {
        return Ok(None);
    }
    let base = configured.unwrap_or_else(|| scratch.join("archon-build-cache"));
    if !base.is_absolute() {
        return Err("ARCHON_CACHE_ROOT/tools.cache_root must be absolute".into());
    }
    let identity = repository
        .canonicalize()
        .unwrap_or_else(|_| repository.into());
    // One directory per checkout, named by a hash of its path. `cache_gc` owns
    // the naming, the marker recording which path that hash stands for, and the
    // lock that keeps a sweep off an entry while it is being built into.
    let store = base.join("unleased");
    let Some((dir, guard)) =
        crate::cache_gc::open_entry(&store, &identity).map_err(|e| format!("cache entry: {e}"))?
    else {
        // A sweep is removing this entry, or its lock cannot be opened. An
        // unlocked entry could be deleted under the build, so this one command
        // runs without the unleased cache rather than into a directory nothing
        // protects.
        tracing::warn!(
            repository = %repository.display(),
            "build cache: unleased entry could not be locked; running without it"
        );
        return Ok(None);
    };
    for (key, value) in crate::build_cache_env::cache_env_for_repository(repository, &dir, extra) {
        if !env.iter().any(|(name, _)| name == &key) {
            std::fs::create_dir_all(&value).map_err(|e| format!("cache directory: {e}"))?;
            env.push((key, value));
        }
    }
    // Swept after this entry is registered and locked, never before, so the
    // sweep can only ever see it as live.
    crate::cache_gc::maybe_sweep(&store);
    Ok(Some(guard))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_routing_keeps_every_ecosystem_below_selected_root() {
        let repo = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        for marker in [
            "Cargo.toml",
            "go.mod",
            "package.json",
            "pyproject.toml",
            "build.gradle",
            "pom.xml",
        ] {
            std::fs::write(repo.path().join(marker), "").unwrap();
        }
        let vars = crate::build_cache_env::cache_env_for_repository(
            repo.path(),
            root.path(),
            &["CUSTOM_CACHE".into()],
        );
        for key in [
            "CARGO_TARGET_DIR",
            "GOCACHE",
            "npm_config_cache",
            "PIP_CACHE_DIR",
            "GRADLE_USER_HOME",
            "MAVEN_OPTS_LOCAL_REPO",
            "CUSTOM_CACHE",
        ] {
            let (_, value) = vars.iter().find(|(name, _)| name == key).unwrap();
            assert!(Path::new(value).starts_with(root.path()));
        }
    }
}
