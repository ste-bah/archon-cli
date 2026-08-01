use std::collections::{BTreeSet, HashMap, HashSet};

use super::catalog_types::{AgentKey, CatalogSnapshot};
use crate::agents::metadata::AgentMetadata;

/// Immutable published view of the discovery catalog.
///
/// The catalog constructs this representation from mutable staging state at
/// publication time. Its fields remain private so a published snapshot cannot
/// be changed through the public API.
#[derive(Clone, Debug, Default)]
pub struct ImmutableCatalogSnapshot {
    entries: HashMap<AgentKey, AgentMetadata>,
    name_index: HashMap<String, BTreeSet<semver::Version>>,
    tag_index: HashMap<String, HashSet<AgentKey>>,
    capability_index: HashMap<String, HashSet<AgentKey>>,
}

impl ImmutableCatalogSnapshot {
    /// Insert one metadata record and update every index.
    pub(crate) fn insert(&mut self, key: AgentKey, metadata: AgentMetadata) {
        self.entries.insert(key.clone(), metadata.clone());
        self.name_index
            .entry(metadata.name.clone())
            .or_default()
            .insert(metadata.version.clone());
        self.add_memberships(&key, &metadata);
    }

    /// Remove memberships owned by one prior metadata record.
    pub(crate) fn remove_memberships(&mut self, key: &AgentKey, metadata: &AgentMetadata) {
        for tag in &metadata.tags {
            Self::remove_membership(&mut self.tag_index, tag, key);
        }
        for capability in &metadata.capabilities {
            Self::remove_membership(&mut self.capability_index, capability, key);
        }
    }

    fn add_memberships(&mut self, key: &AgentKey, metadata: &AgentMetadata) {
        for tag in &metadata.tags {
            self.tag_index
                .entry(tag.clone())
                .or_default()
                .insert(key.clone());
        }
        for capability in &metadata.capabilities {
            self.capability_index
                .entry(capability.clone())
                .or_default()
                .insert(key.clone());
        }
    }

    fn remove_membership(
        index: &mut HashMap<String, HashSet<AgentKey>>,
        membership: &str,
        key: &AgentKey,
    ) {
        if let Some(bucket) = index.get_mut(membership) {
            bucket.remove(key);
            if bucket.is_empty() {
                index.remove(membership);
            }
        }
    }

    /// Create an independent mutable compatibility snapshot.
    pub(crate) fn to_legacy(&self) -> CatalogSnapshot {
        let snapshot = CatalogSnapshot::default();
        for (key, metadata) in &self.entries {
            snapshot.entries.insert(key.clone(), metadata.clone());
        }
        for (name, versions) in &self.name_index {
            snapshot.name_index.insert(name.clone(), versions.clone());
        }
        for (tag, keys) in &self.tag_index {
            snapshot.tag_index.insert(tag.clone(), keys.clone());
        }
        for (capability, keys) in &self.capability_index {
            snapshot
                .capability_index
                .insert(capability.clone(), keys.clone());
        }
        snapshot
    }

    /// Look up one metadata record by its composite key.
    pub fn get(&self, key: &AgentKey) -> Option<&AgentMetadata> {
        self.entries.get(key)
    }

    /// Number of metadata records in this snapshot.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether this snapshot has no metadata records.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over every metadata record and composite key.
    pub fn entries(&self) -> impl Iterator<Item = (&AgentKey, &AgentMetadata)> {
        self.entries.iter()
    }

    /// Iterate over names and their known versions.
    pub fn name_index(&self) -> impl Iterator<Item = (&String, &BTreeSet<semver::Version>)> {
        self.name_index.iter()
    }

    /// Iterate over tags and their metadata keys.
    pub fn tag_index(&self) -> impl Iterator<Item = (&String, &HashSet<AgentKey>)> {
        self.tag_index.iter()
    }

    /// Iterate over capabilities and their metadata keys.
    pub fn capability_index(&self) -> impl Iterator<Item = (&String, &HashSet<AgentKey>)> {
        self.capability_index.iter()
    }

    /// Return the known versions for a name.
    pub fn versions_for(&self, name: &str) -> Option<&BTreeSet<semver::Version>> {
        self.name_index.get(name)
    }

    /// Return the metadata keys carrying a tag.
    pub fn tagged_keys(&self, tag: &str) -> Option<&HashSet<AgentKey>> {
        self.tag_index.get(tag)
    }

    /// Return the metadata keys carrying a capability.
    pub fn capability_keys(&self, capability: &str) -> Option<&HashSet<AgentKey>> {
        self.capability_index.get(capability)
    }
}
