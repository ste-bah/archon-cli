use std::collections::HashSet;

use super::catalog_state::DiscoveryCatalog;
use super::catalog_types::{AgentFilter, AgentInfoView, AgentKey, FilterLogic};
use crate::agents::metadata::{AgentMetadata, AgentState};

#[allow(clippy::result_large_err)]
impl DiscoveryCatalog {
    /// All registered agent names (for suggestions and listing).
    pub fn all_names(&self) -> Vec<String> {
        self.live
            .name_index
            .iter()
            .map(|e| e.key().clone())
            .collect()
    }

    /// List agents matching the given filter.
    ///
    /// Uses tag_index and capability_index for O(1) index lookups,
    /// then intersects (And) or unions (Or) the sets. Name pattern
    /// and version_req are applied as post-filters. Invalid entries
    /// are hidden unless `include_invalid` is set.
    ///
    /// Returns sorted by (name asc, version desc).
    pub fn list(&self, filter: &AgentFilter) -> Vec<AgentMetadata> {
        let candidates = self.filter_candidates(filter);
        let mut results = self.collect_list_results(candidates, filter.include_invalid);
        results.sort_by(|a, b| a.name.cmp(&b.name).then(b.version.cmp(&a.version)));
        results
    }

    fn filter_candidates(&self, filter: &AgentFilter) -> HashSet<AgentKey> {
        let tag_keys = self.indexed_keys(&filter.tags, &filter.logic, |catalog, key| {
            catalog.live.tag_index.get(key).map(|set| set.clone())
        });
        let cap_keys = self.indexed_keys(&filter.capabilities, &filter.logic, |catalog, key| {
            catalog
                .live
                .capability_index
                .get(key)
                .map(|set| set.clone())
        });
        let mut candidates = self.combine_indexed_keys(tag_keys, cap_keys, &filter.logic);
        self.apply_post_filters(&mut candidates, filter);
        candidates
    }

    fn indexed_keys<F>(
        &self,
        keys: &[String],
        logic: &FilterLogic,
        lookup: F,
    ) -> Option<HashSet<AgentKey>>
    where
        F: Fn(&Self, &String) -> Option<HashSet<AgentKey>>,
    {
        (!keys.is_empty()).then(|| {
            let sets = keys
                .iter()
                .map(|key| lookup(self, key).unwrap_or_default())
                .collect::<Vec<_>>();
            combine_sets(&sets, logic)
        })
    }

    fn combine_indexed_keys(
        &self,
        tag_keys: Option<HashSet<AgentKey>>,
        cap_keys: Option<HashSet<AgentKey>>,
        logic: &FilterLogic,
    ) -> HashSet<AgentKey> {
        match (tag_keys, cap_keys) {
            (Some(t), Some(c)) => match logic {
                FilterLogic::And => t.intersection(&c).cloned().collect(),
                FilterLogic::Or => t.union(&c).cloned().collect(),
            },
            (Some(t), None) => t,
            (None, Some(c)) => c,
            (None, None) => self.live.entries.iter().map(|e| e.key().clone()).collect(),
        }
    }

    fn apply_post_filters(&self, candidates: &mut HashSet<AgentKey>, filter: &AgentFilter) {
        if let Some(ref glob) = filter.name_pattern {
            let matcher = glob.compile_matcher();
            candidates.retain(|(name, _)| matcher.is_match(name));
        }
        if let Some(ref req) = filter.version_req {
            candidates.retain(|(_, version)| req.matches(version));
        }
    }

    fn collect_list_results(
        &self,
        candidates: HashSet<AgentKey>,
        include_invalid: bool,
    ) -> Vec<AgentMetadata> {
        candidates
            .iter()
            .filter_map(|key| {
                let entry = self.live.entries.get(key)?;
                let meta = entry.value().clone();
                if !include_invalid && matches!(meta.state, AgentState::Invalid(_)) {
                    return None;
                }
                Some(meta)
            })
            .collect()
    }

    /// Detailed info for a single agent: resolved metadata, all versions, dep graph.
    pub fn info(
        &self,
        name: &str,
        version_req: Option<&semver::VersionReq>,
    ) -> Result<AgentInfoView, super::catalog_types::DiscoveryError> {
        let selected = self.resolve(name, version_req)?;
        let all_versions = self.versions(name);
        let dependency_graph = self.resolve_metadata_dependencies(selected.clone())?;
        Ok(AgentInfoView {
            selected,
            all_versions,
            dependency_graph,
        })
    }

    /// Find the 3 closest name matches using Levenshtein distance.
    pub(super) fn suggest_names(&self, query: &str) -> Vec<String> {
        let mut candidates: Vec<(String, usize)> = self
            .all_names()
            .into_iter()
            .map(|name| {
                let dist = strsim::levenshtein(query, &name);
                (name, dist)
            })
            .filter(|(_, dist)| *dist <= 3)
            .collect();
        candidates.sort_by_key(|(_, dist)| *dist);
        candidates
            .into_iter()
            .take(3)
            .map(|(name, _)| name)
            .collect()
    }
}

/// Combine multiple sets using AND (intersection) or OR (union) logic.
fn combine_sets(sets: &[HashSet<AgentKey>], logic: &FilterLogic) -> HashSet<AgentKey> {
    if sets.is_empty() {
        return HashSet::new();
    }
    match logic {
        FilterLogic::And => {
            let mut result = sets[0].clone();
            for set in &sets[1..] {
                result = result.intersection(set).cloned().collect();
            }
            result
        }
        FilterLogic::Or => {
            let mut result = HashSet::new();
            for set in sets {
                result = result.union(set).cloned().collect();
            }
            result
        }
    }
}
