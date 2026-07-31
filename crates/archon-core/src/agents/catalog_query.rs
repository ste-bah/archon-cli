use std::collections::HashSet;

use super::catalog_resolution::{
    resolve_metadata_dependencies_snapshot, resolve_snapshot, versions_snapshot,
};
use super::catalog_state::DiscoveryCatalog;
use super::catalog_types::{AgentFilter, AgentInfoView, AgentKey, CatalogSnapshot, FilterLogic};
use crate::agents::metadata::{AgentMetadata, AgentState};

impl DiscoveryCatalog {
    /// All registered agent names (for suggestions and listing).
    pub fn all_names(&self) -> Vec<String> {
        let snapshot = self.snapshot();
        snapshot
            .name_index
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// List agents matching the given filter, sorted by name then version.
    pub fn list(&self, filter: &AgentFilter) -> Vec<AgentMetadata> {
        let snapshot = self.snapshot();
        let candidates = filter_candidates(&snapshot, filter);
        let mut results = collect_list_results(&snapshot, candidates, filter.include_invalid);
        results.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then(right.version.cmp(&left.version))
        });
        results
    }

    /// Detailed info for a single agent from one complete snapshot.
    pub fn info(
        &self,
        name: &str,
        version_req: Option<&semver::VersionReq>,
    ) -> Result<AgentInfoView, super::catalog_types::DiscoveryError> {
        let snapshot = self.snapshot();
        let selected = resolve_snapshot(&snapshot, name, version_req)?;
        let all_versions = versions_snapshot(&snapshot, name);
        let dependency_graph = resolve_metadata_dependencies_snapshot(&snapshot, selected.clone())?;
        Ok(AgentInfoView {
            selected,
            all_versions,
            dependency_graph,
        })
    }
}

fn filter_candidates(snapshot: &CatalogSnapshot, filter: &AgentFilter) -> HashSet<AgentKey> {
    let tags = indexed_keys(&snapshot.tag_index, &filter.tags, &filter.logic);
    let capabilities = indexed_keys(
        &snapshot.capability_index,
        &filter.capabilities,
        &filter.logic,
    );
    let mut candidates = combine_indexed_keys(snapshot, tags, capabilities, &filter.logic);
    apply_post_filters(&mut candidates, filter);
    candidates
}

fn indexed_keys(
    index: &dashmap::DashMap<String, HashSet<AgentKey>>,
    keys: &[String],
    logic: &FilterLogic,
) -> Option<HashSet<AgentKey>> {
    (!keys.is_empty()).then(|| {
        let sets = keys
            .iter()
            .map(|key| index.get(key).map(|set| set.clone()).unwrap_or_default())
            .collect::<Vec<_>>();
        combine_sets(&sets, logic)
    })
}

fn combine_indexed_keys(
    snapshot: &CatalogSnapshot,
    tags: Option<HashSet<AgentKey>>,
    capabilities: Option<HashSet<AgentKey>>,
    logic: &FilterLogic,
) -> HashSet<AgentKey> {
    match (tags, capabilities) {
        (Some(tags), Some(capabilities)) if matches!(logic, FilterLogic::And) => {
            tags.intersection(&capabilities).cloned().collect()
        }
        (Some(tags), Some(capabilities)) => tags.union(&capabilities).cloned().collect(),
        (Some(keys), None) | (None, Some(keys)) => keys,
        (None, None) => snapshot
            .entries
            .iter()
            .map(|entry| entry.key().clone())
            .collect(),
    }
}

fn apply_post_filters(candidates: &mut HashSet<AgentKey>, filter: &AgentFilter) {
    if let Some(glob) = &filter.name_pattern {
        let matcher = glob.compile_matcher();
        candidates.retain(|(name, _)| matcher.is_match(name));
    }
    if let Some(requirement) = &filter.version_req {
        candidates.retain(|(_, version)| requirement.matches(version));
    }
}

fn collect_list_results(
    snapshot: &CatalogSnapshot,
    candidates: HashSet<AgentKey>,
    include_invalid: bool,
) -> Vec<AgentMetadata> {
    candidates
        .iter()
        .filter_map(|key| snapshot.entries.get(key).map(|entry| entry.value().clone()))
        .filter(|metadata| include_invalid || !matches!(metadata.state, AgentState::Invalid(_)))
        .collect()
}

/// Combine multiple sets using AND (intersection) or OR (union) logic.
fn combine_sets(sets: &[HashSet<AgentKey>], logic: &FilterLogic) -> HashSet<AgentKey> {
    let Some(first) = sets.first() else {
        return HashSet::new();
    };
    sets.iter()
        .skip(1)
        .fold(first.clone(), |result, set| match logic {
            FilterLogic::And => result.intersection(set).cloned().collect(),
            FilterLogic::Or => result.union(set).cloned().collect(),
        })
}
