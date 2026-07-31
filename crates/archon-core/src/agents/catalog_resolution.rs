use std::collections::HashSet;

use crate::agents::metadata::{AgentMetadata, AgentState};

use super::catalog_state::DiscoveryCatalog;
use super::catalog_types::{AgentKey, DiscoveryError};

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
        let mut resolved = Vec::new();
        let mut visiting = HashSet::new();
        let mut path = Vec::new();
        self.dfs_dependencies(root_name, &mut resolved, &mut visiting, &mut path)?;
        Ok(resolved)
    }

    fn dfs_dependencies(
        &self,
        name: &str,
        resolved: &mut Vec<AgentKey>,
        visiting: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> Result<(), DiscoveryError> {
        if visiting.contains(name) {
            path.push(name.to_string());
            return Err(DiscoveryError::CircularDependency(path.clone()));
        }

        visiting.insert(name.to_string());
        path.push(name.to_string());
        if let Ok(meta) = self.resolve(name, None) {
            for dep in &meta.dependencies {
                let dep_req = &dep.version_req;
                if let Ok(dep_meta) = self.resolve(&dep.name, Some(dep_req)) {
                    let dep_key = (dep_meta.name.clone(), dep_meta.version.clone());
                    if !resolved.contains(&dep_key) {
                        self.dfs_dependencies(&dep.name, resolved, visiting, path)?;
                        resolved.push(dep_key);
                    }
                }
            }
        }

        visiting.remove(name);
        path.pop();
        Ok(())
    }
}
