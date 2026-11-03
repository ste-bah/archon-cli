//! Single source for host authentication stores, including atomic-write siblings.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

/// Environment override for the Archon credentials file path.
pub const CREDENTIALS_FILE_ENV: &str = "ARCHON_CREDENTIALS_FILE";
/// Environment override for the Codex CLI authentication file path.
pub const CODEX_AUTH_FILE_ENV: &str = "ARCHON_CODEX_AUTH_FILE";

static REGISTERED: OnceLock<RwLock<BTreeSet<PathBuf>>> = OnceLock::new();

/// Host credential readers/writers register custom locations before exposing
/// any agent launch. This records paths only, never secret values.
pub fn register_store(path: &Path) {
    let path = absolute(path);
    let mut paths = REGISTERED
        .get_or_init(Default::default)
        .write()
        .unwrap_or_else(|e| e.into_inner());
    paths.insert(atomic_sibling(&path));
    paths.insert(path);
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub fn host_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("archon")
}

pub fn host_config() -> PathBuf {
    host_config_dir().join("config.toml")
}
pub fn remote_token() -> PathBuf {
    host_config_dir().join("remote-token")
}

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Host-owned run grant authority, resolved like the default credential root.
pub fn operator_grant_root() -> PathBuf {
    home().join(".archon/operator-env-grants")
}

pub fn default_provider_store() -> PathBuf {
    default_provider_store_in(&home())
}

pub fn default_provider_store_in(home: &Path) -> PathBuf {
    let current = home.join(".archon/.credentials.json");
    let legacy = home.join(".claude/.credentials.json");
    if current.exists() {
        return current;
    }
    if legacy.exists() {
        tracing::warn!(
            "Loading from deprecated path {}. Rename to {} to suppress this warning.",
            legacy.display(),
            current.display()
        );
        return legacy;
    }
    // Neither exists — return the new path (callers check existence)
    current
}

// Explicit overrides are honoured in real binaries too. On Windows the
// home resolver uses the known-folder API, not HOME, so launch tests and
// operators need these existing overrides to move their authentication store.
// Explicit override, honoured in real binaries as well as tests.
//
// Everything below resolves from `dirs::home_dir()`, which on Windows comes
// from the shell known-folder API rather than an environment variable. A
// spawned `archon` therefore ignored `HOME` entirely, so integration tests
// could not point it at a scratch profile -- and users had no way to move
// the credential file at all. The `#[cfg(test)]` hook above only ever
// helped in-process unit tests.
pub fn provider_store() -> PathBuf {
    override_path(CREDENTIALS_FILE_ENV).unwrap_or_else(default_provider_store)
}

// Explicit override, for the same reason as `ARCHON_CREDENTIALS_FILE`:
// `dirs::home_dir()` reads the shell known-folder API on Windows, not
// `HOME`, so a spawned `archon` could not be pointed at a scratch profile.
// Worse than a red test -- it meant the auth-status tests read whatever
// Codex credentials the developer actually had installed.
pub fn codex_cli_store() -> PathBuf {
    override_path(CODEX_AUTH_FILE_ENV).unwrap_or_else(|| home().join(".codex/auth.json"))
}

fn override_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Both OAuth writers use this sibling before their atomic rename.
pub fn atomic_sibling(path: &Path) -> PathBuf {
    path.with_extension("json.tmp")
}

pub fn stores_in(home: &Path) -> Vec<PathBuf> {
    [
        ".archon/.credentials.json",
        ".claude/.credentials.json",
        ".codex/auth.json",
    ]
    .iter()
    .map(|p| home.join(p))
    .collect()
}

pub fn protected_paths() -> Vec<PathBuf> {
    let mut paths = stores_in(&home());
    paths.extend([
        provider_store(),
        default_provider_store(),
        codex_cli_store(),
        remote_token(),
        host_config(),
        operator_grant_root(),
        host_config_dir().join(".mcp.json"),
    ]);
    let siblings: Vec<_> = paths.iter().map(|p| atomic_sibling(p)).collect();
    paths.extend(siblings);
    paths.extend(
        REGISTERED
            .get_or_init(Default::default)
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned(),
    );
    paths = paths.iter().map(|path| absolute(path)).collect();
    paths.sort();
    paths.dedup();
    paths
}
