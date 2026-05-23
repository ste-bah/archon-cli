//! TOML policy layer for Evidence Engine gates.

pub mod decision;
pub mod errors;
pub mod loader;
pub mod models;
mod video;

use std::path::Path;

pub use errors::{PolicyError, Result};
pub use loader::{PolicyLoad, PolicySource, load_policy_for_workspace, load_policy_from_sources};
pub use models::*;

pub fn load_effective_policy(workspace_dir: &Path) -> Result<EffectivePolicy> {
    Ok(load_policy_for_workspace(workspace_dir)?.policy)
}
