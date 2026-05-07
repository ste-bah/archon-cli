use std::path::{Path, PathBuf};

use super::super::errors::GameTheoryError;
use super::types::GameTheorySpec;

/// Load the gametheory spec from the canonical YAML path.
pub fn load_spec(path: &Path) -> Result<GameTheorySpec, GameTheoryError> {
    let contents = std::fs::read_to_string(path).map_err(|e| GameTheoryError::Io {
        message: format!("cannot read spec at {}: {e}", path.display()),
    })?;
    serde_yml::from_str(&contents).map_err(|e| GameTheoryError::Validation {
        message: format!("invalid gametheory spec YAML: {e}"),
    })
}

/// Resolve the gametheory spec path by searching known locations.
///
/// Search order:
/// 1. Explicit `--spec-path` CLI flag (passed as `explicit_path`)
/// 2. `$ARCHON_SPEC_PATH` environment variable
/// 3. Walk up from CWD looking for `.archon/specs/gametheory.yaml` (max 5 levels)
/// 4. `~/.archon/specs/gametheory.yaml` (user install)
/// 5. `/etc/archon/specs/gametheory.yaml` (system install)
///
/// Returns the first path that exists, or a `SpecNotFound` error listing all
/// locations searched.
pub fn resolve_spec_path(explicit_path: Option<&Path>) -> Result<PathBuf, GameTheoryError> {
    let mut searched = Vec::new();

    // 1. Explicit CLI flag
    if let Some(p) = explicit_path {
        searched.push(p.to_path_buf());
        if p.exists() {
            return Ok(p.to_path_buf());
        }
    }

    // 2. Env var
    if let Ok(env_path) = std::env::var("ARCHON_SPEC_PATH") {
        let p = PathBuf::from(&env_path);
        searched.push(p.clone());
        if p.exists() {
            return Ok(p);
        }
    }

    // 3. Walk up from CWD (max 5 levels)
    if let Ok(cwd) = std::env::current_dir() {
        let mut current = cwd.as_path();
        for _ in 0..5 {
            let candidate = current.join(".archon/specs/gametheory.yaml");
            searched.push(candidate.clone());
            if candidate.exists() {
                return Ok(candidate);
            }
            match current.parent() {
                Some(parent) => current = parent,
                None => break,
            }
        }
    }

    // 4. User install
    if let Ok(home) = std::env::var("HOME") {
        let user_path = PathBuf::from(&home).join(".archon/specs/gametheory.yaml");
        searched.push(user_path.clone());
        if user_path.exists() {
            return Ok(user_path);
        }
    }

    // 5. System install
    let system_path = PathBuf::from("/etc/archon/specs/gametheory.yaml");
    searched.push(system_path.clone());
    if system_path.exists() {
        return Ok(system_path);
    }

    Err(GameTheoryError::SpecNotFound {
        searched_paths: searched,
    })
}
