use std::collections::HashSet;

use crate::agents::metadata::{AgentMetadata, AgentState};

use super::catalog_state::DiscoveryCatalog;
use super::catalog_types::{AgentKey, DiscoveryError};

#[allow(clippy::result_large_err)]
impl DiscoveryCatalog {
    /// Resolve the best matching agent by name and optional version requirement.
    ///
    /// - `version_req = None` → highest Valid version.
    /// - `version_req = Some(req)` → highest Valid version matching the req.
    /// - Unknown name → `AgentNotFound` with Levenshtein-based suggestions.
    pub fn resolve(
        &self,
        name: &str,
        version_req: Option<&semver::VersionReq>,
    ) -> Result<AgentMetadata, DiscoveryError> {
        let versions_set = self.live.name_index.get(name);
        let versions_set = match versions_set {
            Some(v) => v,
            None => {
                return Err(DiscoveryError::AgentNotFound {
                    name: name.to_string(),
                    suggestions: self.suggest_names(name),
                });
            }
        };

        for version in versions_set.iter().rev() {
            if let Some(req) = version_req
                && !req.matches(version)
            {
                continue;
            }
            let key = (name.to_string(), version.clone());
            if let Some(entry) = self.live.entries.get(&key)
                && matches!(entry.state, AgentState::Valid)
            {
                return Ok(entry.value().clone());
            }
        }

        Err(DiscoveryError::AgentNotFound {
            name: name.to_string(),
            suggestions: self.suggest_names(name),
        })
    }

    /// Return all known versions for a name, sorted descending.
    pub fn versions(&self, name: &str) -> Vec<semver::Version> {
        self.live
            .name_index
            .get(name)
            .map(|set| set.iter().rev().cloned().collect())
            .unwrap_or_default()
    }

    /// Resolve transitive dependencies via DFS.
    /// Returns `CircularDependency` on cycle detection.
    pub fn resolve_dependencies(&self, root_name: &str) -> Result<Vec<AgentKey>, DiscoveryError> {
        let root = self.resolve(root_name, None)?;
        self.resolve_metadata_dependencies(root)
    }

    pub(super) fn resolve_metadata_dependencies(
        &self,
        root: AgentMetadata,
    ) -> Result<Vec<AgentKey>, DiscoveryError> {
        let mut resolved = Vec::new();
        let mut visiting = HashSet::new();
        let mut path = Vec::new();
        self.dfs_dependencies(root, &mut resolved, &mut visiting, &mut path)?;
        Ok(resolved)
    }

    fn dfs_dependencies(
        &self,
        meta: AgentMetadata,
        resolved: &mut Vec<AgentKey>,
        visiting: &mut HashSet<AgentKey>,
        path: &mut Vec<String>,
    ) -> Result<(), DiscoveryError> {
        let key = (meta.name.clone(), meta.version.clone());
        if visiting.contains(&key) {
            path.push(meta.name);
            return Err(DiscoveryError::CircularDependency(path.clone()));
        }

        visiting.insert(key.clone());
        path.push(meta.name.clone());
        for dep in &meta.dependencies {
            let dep_meta = self
                .resolve(&dep.name, Some(&dep.version_req))
                .map_err(|_| DiscoveryError::UnresolvedDependency {
                    required_by: key.clone(),
                    name: dep.name.clone(),
                    version_req: dep.version_req.clone(),
                    suggestions: self.suggest_names(&dep.name),
                })?;
            let dep_key = (dep_meta.name.clone(), dep_meta.version.clone());
            if !resolved.contains(&dep_key) {
                self.dfs_dependencies(dep_meta, resolved, visiting, path)?;
                resolved.push(dep_key);
            }
        }

        visiting.remove(&key);
        path.pop();
        Ok(())
    }
}
