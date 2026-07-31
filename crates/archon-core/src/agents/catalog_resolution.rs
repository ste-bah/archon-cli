use std::collections::HashSet;

use crate::agents::metadata::{AgentMetadata, AgentState};

use super::catalog_state::DiscoveryCatalog;
use super::catalog_types::{AgentKey, CatalogSnapshot, DiscoveryError, UnresolvedDependency};

impl DiscoveryCatalog {
    /// Resolve the best matching valid agent by name and version requirement.
    pub fn resolve(
        &self,
        name: &str,
        version_req: Option<&semver::VersionReq>,
    ) -> Result<AgentMetadata, DiscoveryError> {
        let snapshot = self.snapshot();
        resolve_snapshot(&snapshot, name, version_req)
    }

    /// Return all known versions for a name, sorted descending.
    pub fn versions(&self, name: &str) -> Vec<semver::Version> {
        let snapshot = self.snapshot();
        versions_snapshot(&snapshot, name)
    }

    /// Resolve transitive dependencies via DFS.
    pub fn resolve_dependencies(&self, root_name: &str) -> Result<Vec<AgentKey>, DiscoveryError> {
        let snapshot = self.snapshot();
        let root = resolve_snapshot(&snapshot, root_name, None)?;
        resolve_metadata_dependencies_snapshot(&snapshot, root)
    }
}

pub(super) fn resolve_snapshot(
    snapshot: &CatalogSnapshot,
    name: &str,
    version_req: Option<&semver::VersionReq>,
) -> Result<AgentMetadata, DiscoveryError> {
    let Some(versions) = snapshot.name_index.get(name) else {
        return Err(not_found(snapshot, name));
    };
    for version in versions.iter().rev() {
        if version_req.is_some_and(|requirement| !requirement.matches(version)) {
            continue;
        }
        if let Some(entry) = snapshot.entries.get(&(name.to_string(), version.clone()))
            && matches!(entry.state, AgentState::Valid)
        {
            return Ok(entry.value().clone());
        }
    }
    Err(not_found(snapshot, name))
}

pub(super) fn versions_snapshot(snapshot: &CatalogSnapshot, name: &str) -> Vec<semver::Version> {
    snapshot
        .name_index
        .get(name)
        .map(|versions| versions.iter().rev().cloned().collect())
        .unwrap_or_default()
}

fn not_found(snapshot: &CatalogSnapshot, name: &str) -> DiscoveryError {
    let mut suggestions: Vec<_> = snapshot
        .name_index
        .iter()
        .map(|entry| entry.key().clone())
        .map(|candidate| (candidate.clone(), strsim::levenshtein(name, &candidate)))
        .filter(|(_, distance)| *distance <= 3)
        .collect();
    suggestions.sort_by_key(|(_, distance)| *distance);
    DiscoveryError::AgentNotFound {
        name: name.to_string(),
        suggestions: suggestions
            .into_iter()
            .take(3)
            .map(|(candidate, _)| candidate)
            .collect(),
    }
}

pub(super) fn resolve_metadata_dependencies_snapshot(
    snapshot: &CatalogSnapshot,
    root: AgentMetadata,
) -> Result<Vec<AgentKey>, DiscoveryError> {
    let mut resolved = Vec::new();
    let mut visiting = HashSet::new();
    let mut path = Vec::new();
    dfs_dependencies(snapshot, root, &mut resolved, &mut visiting, &mut path)?;
    Ok(resolved)
}

fn dfs_dependencies(
    snapshot: &CatalogSnapshot,
    meta: AgentMetadata,
    resolved: &mut Vec<AgentKey>,
    visiting: &mut HashSet<AgentKey>,
    path: &mut Vec<String>,
) -> Result<(), DiscoveryError> {
    let key = (meta.name.clone(), meta.version.clone());
    if !visiting.insert(key.clone()) {
        path.push(meta.name);
        return Err(DiscoveryError::CircularDependency(path.clone()));
    }
    path.push(meta.name.clone());
    for dependency in &meta.dependencies {
        let dependency_meta =
            resolve_snapshot(snapshot, &dependency.name, Some(&dependency.version_req))
                .map_err(|error| unresolved_dependency(error, &key, dependency))?;
        let dependency_key = (
            dependency_meta.name.clone(),
            dependency_meta.version.clone(),
        );
        if !resolved.contains(&dependency_key) {
            dfs_dependencies(snapshot, dependency_meta, resolved, visiting, path)?;
            resolved.push(dependency_key);
        }
    }
    visiting.remove(&key);
    path.pop();
    Ok(())
}

fn unresolved_dependency(
    error: DiscoveryError,
    required_by: &AgentKey,
    dependency: &crate::agents::metadata::DependencyRef,
) -> DiscoveryError {
    match error {
        DiscoveryError::AgentNotFound { suggestions, .. } => {
            DiscoveryError::UnresolvedDependency(Box::new(UnresolvedDependency {
                required_by: required_by.clone(),
                name: dependency.name.clone(),
                version_req: dependency.version_req.clone(),
                suggestions,
            }))
        }
        other => other,
    }
}
