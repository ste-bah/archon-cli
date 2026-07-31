use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use tracing::warn;

use super::catalog_types::{AgentKey, CatalogSnapshot, DiscoveryError};
use crate::agents::metadata::AgentMetadata;

/// In-memory catalog with a serialized staging state and published snapshots.
pub struct DiscoveryCatalog {
    staging: Mutex<CatalogSnapshot>,
    cached_snapshot: ArcSwap<CatalogSnapshot>,
}

/// The accepted and rejected records from one bulk insertion.
#[derive(Debug, Default)]
pub struct BulkInsertResult {
    pub accepted: AcceptedInsertCounts,
    pub rejected: Vec<BulkInsertRejection>,
}

/// State counts for records accepted by one bulk insertion.
#[derive(Debug, Default)]
pub struct AcceptedInsertCounts {
    pub loaded: usize,
    pub invalid: usize,
}

/// A metadata record rejected from one bulk insertion with its cause.
#[derive(Debug)]
pub struct BulkInsertRejection {
    pub metadata: AgentMetadata,
    pub error: DiscoveryError,
}

fn count_accepted_metadata(counts: &mut AcceptedInsertCounts, meta: &AgentMetadata) {
    match &meta.state {
        crate::agents::metadata::AgentState::Valid | crate::agents::metadata::AgentState::Stale => {
            counts.loaded += 1;
        }
        crate::agents::metadata::AgentState::Invalid(_) => counts.invalid += 1,
    }
}

fn decrement_accepted_count(counts: &mut AcceptedInsertCounts, meta: &AgentMetadata) {
    match &meta.state {
        crate::agents::metadata::AgentState::Valid | crate::agents::metadata::AgentState::Stale => {
            counts.loaded -= 1;
        }
        crate::agents::metadata::AgentState::Invalid(_) => counts.invalid -= 1,
    }
}

fn metadata_key(meta: &AgentMetadata) -> AgentKey {
    (meta.name.clone(), meta.version.clone())
}

impl DiscoveryCatalog {
    /// Create an empty catalog.
    pub fn new() -> Self {
        Self {
            staging: Mutex::new(CatalogSnapshot::default()),
            cached_snapshot: ArcSwap::from_pointee(CatalogSnapshot::default()),
        }
    }

    /// Insert an entry and publish the resulting complete snapshot.
    pub fn insert(&self, meta: AgentMetadata) -> Result<(), DiscoveryError> {
        self.validate_metadata(&meta)?;
        let mut staging = self.staging.lock().expect("catalog staging lock poisoned");
        self.insert_staged(&mut staging, meta);
        self.publish(&staging);
        Ok(())
    }

    /// Insert entries under one writer lock and publish once for bulk discovery.
    /// Invalid entries are rejected individually; accepted entries share one publication.
    pub fn insert_all(&self, entries: Vec<AgentMetadata>) -> BulkInsertResult {
        let mut accepted = Vec::with_capacity(entries.len());
        let mut rejected = Vec::new();
        let mut accepted_counts = AcceptedInsertCounts::default();
        for meta in entries {
            match self.validate_metadata(&meta) {
                Ok(()) => {
                    count_accepted_metadata(&mut accepted_counts, &meta);
                    accepted.push(meta);
                }
                Err(error) => rejected.push(BulkInsertRejection {
                    metadata: meta,
                    error,
                }),
            }
        }
        if !accepted.is_empty() {
            let mut staging = self.staging.lock().expect("catalog staging lock poisoned");
            for meta in accepted {
                match self.insert_bulk_staged(&mut staging, meta) {
                    Ok(()) => {}
                    Err(rejected_insert) => {
                        decrement_accepted_count(&mut accepted_counts, &rejected_insert.metadata);
                        rejected.push(*rejected_insert);
                    }
                }
            }
            self.publish(&staging);
        }
        BulkInsertResult {
            accepted: accepted_counts,
            rejected,
        }
    }

    fn validate_metadata(&self, meta: &AgentMetadata) -> Result<(), DiscoveryError> {
        let serialized =
            serde_json::to_vec(meta).map_err(|e| DiscoveryError::Parse(e.to_string()))?;
        if serialized.len() > 10 * 1024 * 1024 {
            return Err(DiscoveryError::MetadataTooLarge {
                path: meta.source_path.clone(),
                size: serialized.len(),
            });
        }
        Ok(())
    }

    fn insert_staged(&self, staging: &mut CatalogSnapshot, meta: AgentMetadata) {
        if let Some(existing) = staging
            .entries
            .get(&metadata_key(&meta))
            .map(|entry| entry.value().clone())
        {
            if existing.source_path != meta.source_path {
                warn!(
                    "agent collision: name={} version={} existing={:?} ignored={:?}",
                    meta.name, meta.version, existing.source_path, meta.source_path
                );
                return;
            }
            Self::remove_memberships(staging, &metadata_key(&meta), &existing);
        }
        Self::store_staged(staging, meta);
    }

    /// Inserts bulk metadata while preserving rejected contenders for reporting.
    fn insert_bulk_staged(
        &self,
        staging: &mut CatalogSnapshot,
        meta: AgentMetadata,
    ) -> Result<(), Box<BulkInsertRejection>> {
        if let Some(existing) = staging
            .entries
            .get(&metadata_key(&meta))
            .map(|entry| entry.value().clone())
        {
            if existing.source_path != meta.source_path {
                return Err(Box::new(BulkInsertRejection {
                    error: DiscoveryError::DuplicateAgent {
                        name: meta.name.clone(),
                        version: meta.version.clone(),
                        existing_path: existing.source_path,
                        rejected_path: meta.source_path.clone(),
                    },
                    metadata: meta,
                }));
            }
            Self::remove_memberships(staging, &metadata_key(&meta), &existing);
        }
        Self::store_staged(staging, meta);
        Ok(())
    }

    fn store_staged(staging: &mut CatalogSnapshot, meta: AgentMetadata) {
        let key = metadata_key(&meta);
        staging.entries.insert(key.clone(), meta.clone());
        staging
            .name_index
            .entry(meta.name.clone())
            .or_default()
            .insert(meta.version.clone());
        Self::add_memberships(staging, &key, &meta);
    }

    fn remove_memberships(staging: &mut CatalogSnapshot, key: &AgentKey, meta: &AgentMetadata) {
        for tag in &meta.tags {
            Self::remove_membership(&staging.tag_index, tag, key);
        }
        for capability in &meta.capabilities {
            Self::remove_membership(&staging.capability_index, capability, key);
        }
    }

    fn remove_membership(
        index: &dashmap::DashMap<String, std::collections::HashSet<AgentKey>>,
        membership: &str,
        key: &AgentKey,
    ) {
        if let Some(mut bucket) = index.get_mut(membership) {
            bucket.remove(key);
            if bucket.is_empty() {
                drop(bucket);
                index.remove(membership);
            }
        }
    }

    fn add_memberships(staging: &mut CatalogSnapshot, key: &AgentKey, meta: &AgentMetadata) {
        for tag in &meta.tags {
            staging
                .tag_index
                .entry(tag.clone())
                .or_default()
                .insert(key.clone());
        }
        for capability in &meta.capabilities {
            staging
                .capability_index
                .entry(capability.clone())
                .or_default()
                .insert(key.clone());
        }
    }

    fn publish(&self, staging: &CatalogSnapshot) {
        self.cached_snapshot.store(Arc::new(staging.clone()));
    }

    /// Look up a specific entry from one published snapshot.
    pub fn get(&self, key: &AgentKey) -> Option<AgentMetadata> {
        self.snapshot()
            .entries
            .get(key)
            .map(|entry| entry.value().clone())
    }

    /// Return the current complete catalog snapshot.
    pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
        self.cached_snapshot.load_full()
    }

    /// Number of entries in one published snapshot.
    pub fn len(&self) -> usize {
        self.snapshot().entries.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.snapshot().entries.is_empty()
    }
}

impl Default for DiscoveryCatalog {
    fn default() -> Self {
        Self::new()
    }
}
