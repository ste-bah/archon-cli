//! Catalog representation benchmark for Issue #109.
//!
//! This harness compares the current `DashMap`-backed `CatalogSnapshot` with an
//! equivalent immutable representation built from standard `HashMap`s. It measures
//! complete publication plus production-equivalent exact, resolution, and indexed
//! read facades. Fixtures, `ArcSwap` targets, and equivalence checks are ready
//! before Criterion begins timing.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use arc_swap::ArcSwap;
use archon_bench::catalog_representation::{deterministic_index_checksum, metadata_digest};
use archon_core::agents::{
    AgentKey, AgentMetadata, AgentState, CatalogSnapshot, DiscoveryCatalog,
    ImmutableCatalogSnapshot, ResourceReq, SourceKind,
};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

const FIXTURE_SIZES: [usize; 3] = [100, 1_000, 10_000];
const VERSIONS_PER_NAME: usize = 4;

struct DashReadFacade {
    snapshot: ArcSwap<CatalogSnapshot>,
}

struct StandardReadFacade {
    snapshot: ArcSwap<ImmutableCatalogSnapshot>,
}

struct Fixture {
    dash: CatalogSnapshot,
    standard: ImmutableCatalogSnapshot,
    dash_reads: DashReadFacade,
    standard_reads: StandardReadFacade,
    exact_key: AgentKey,
    lookup_name: String,
    tag: String,
    capability: String,
}

#[derive(Debug, Eq, PartialEq)]
struct ResultChecks {
    exact: u64,
    highest_version: u64,
    indexed_count: usize,
    indexed_checksum: u64,
}

impl Fixture {
    fn new(agent_count: usize) -> Self {
        assert!(agent_count >= VERSIONS_PER_NAME);
        let (dash, standard) = build_snapshots(agent_count);
        let representative = agent_count / 2 / VERSIONS_PER_NAME;
        let fixture = Self {
            dash_reads: DashReadFacade::from_snapshot(&dash),
            standard_reads: StandardReadFacade::from_snapshot(&standard),
            dash,
            standard,
            exact_key: (
                format!("agent-{representative}"),
                semver::Version::new(1, 0, 0),
            ),
            lookup_name: format!("agent-{representative}"),
            tag: "tag-2".to_owned(),
            capability: "capability-3".to_owned(),
        };
        fixture.validate_equivalence();
        fixture.complete_clone_matches_standard();
        fixture
    }

    fn validate_equivalence(&self) {
        assert_eq!(
            entry_digests_dash(&self.dash),
            entry_digests_standard(&self.standard)
        );
        assert_eq!(snapshot_digest(&self.dash), standard_digest(&self.standard));
        assert_eq!(
            name_index_checksum_dash(&self.dash),
            name_index_checksum_standard(&self.standard)
        );
        assert_eq!(
            membership_index_checksum_dash(&self.dash.tag_index),
            membership_index_checksum_standard(self.standard.tag_index())
        );
        assert_eq!(
            membership_index_checksum_dash(&self.dash.capability_index),
            membership_index_checksum_standard(self.standard.capability_index()),
            "capability index checksum"
        );
        assert_eq!(dash_results(self), standard_results(self));
    }

    fn complete_clone_matches_standard(&self) {
        let dash = self.dash.clone();
        let standard = self.standard.clone();
        assert_eq!(entry_digests_dash(&dash), entry_digests_standard(&standard));
        assert_eq!(
            membership_index_checksum_dash(&dash.capability_index),
            membership_index_checksum_standard(standard.capability_index()),
            "complete capability index checksum"
        );
    }
}

impl DashReadFacade {
    fn from_snapshot(snapshot: &CatalogSnapshot) -> Self {
        Self {
            snapshot: ArcSwap::from(Arc::new(snapshot.clone())),
        }
    }

    fn exact_get(&self, key: &AgentKey) -> AgentMetadata {
        let snapshot = self.snapshot.load_full();
        exact_dash(&snapshot, key)
    }

    fn highest_version(&self, name: &str) -> AgentMetadata {
        let snapshot = self.snapshot.load_full();
        highest_dash(&snapshot, name)
    }

    fn indexed_read(&self, tag: &str, capability: &str) -> Vec<AgentMetadata> {
        let snapshot = self.snapshot.load_full();
        indexed_dash(&snapshot, tag, capability)
    }
}

impl StandardReadFacade {
    fn from_snapshot(snapshot: &ImmutableCatalogSnapshot) -> Self {
        Self {
            snapshot: ArcSwap::from(Arc::new(snapshot.clone())),
        }
    }

    fn exact_get(&self, key: &AgentKey) -> AgentMetadata {
        let snapshot = self.snapshot.load_full();
        exact_standard(&snapshot, key)
    }

    fn highest_version(&self, name: &str) -> AgentMetadata {
        let snapshot = self.snapshot.load_full();
        highest_standard(&snapshot, name)
    }

    fn indexed_read(&self, tag: &str, capability: &str) -> Vec<AgentMetadata> {
        let snapshot = self.snapshot.load_full();
        indexed_standard(&snapshot, tag, capability)
    }
}

fn build_snapshots(agent_count: usize) -> (CatalogSnapshot, ImmutableCatalogSnapshot) {
    let metadata: Vec<_> = (0..agent_count).map(fixture_metadata).collect();
    let dash = build_dash_snapshot(&metadata);
    let catalog = DiscoveryCatalog::new();
    let result = catalog.insert_all(metadata);
    assert!(result.rejected.is_empty(), "benchmark fixture rejections");
    assert_eq!(result.accepted.loaded, agent_count);
    let standard = catalog.snapshot_immutable().as_ref().clone();
    (dash, standard)
}

fn build_dash_snapshot(metadata: &[AgentMetadata]) -> CatalogSnapshot {
    let mut dash = CatalogSnapshot::default();
    for metadata in metadata {
        let key = (metadata.name.clone(), metadata.version.clone());
        insert_dash(&mut dash, key, metadata.clone());
    }
    dash
}

fn fixture_metadata(index: usize) -> AgentMetadata {
    let version = semver::Version::new(1, 0, (index % VERSIONS_PER_NAME) as u64);
    AgentMetadata {
        name: format!("agent-{}", index / VERSIONS_PER_NAME),
        version,
        description: format!("deterministic fixture agent {index}"),
        category: "benchmark".to_owned(),
        tags: vec![format!("tag-{}", index % 5), format!("group-{}", index % 7)],
        capabilities: vec![
            format!("capability-{}", index % 7),
            format!("feature-{}", index % 11),
        ],
        input_schema: serde_json::json!({"type": "object", "fixture": index}),
        output_schema: serde_json::json!({"type": "object", "result": index}),
        resource_requirements: ResourceReq::default(),
        dependencies: Vec::new(),
        source_path: format!("/fixtures/agent-{index}.toml").into(),
        source_kind: SourceKind::Local,
        state: AgentState::Valid,
        loaded_at: chrono::DateTime::from_timestamp(1_700_000_000, 0)
            .expect("fixed fixture timestamp"),
    }
}

fn insert_dash(snapshot: &mut CatalogSnapshot, key: AgentKey, metadata: AgentMetadata) {
    snapshot.entries.insert(key.clone(), metadata.clone());
    snapshot
        .name_index
        .entry(metadata.name.clone())
        .or_default()
        .insert(metadata.version.clone());
    insert_memberships_dash(&snapshot.tag_index, &metadata.tags, &key);
    insert_memberships_dash(&snapshot.capability_index, &metadata.capabilities, &key);
}

fn insert_memberships_dash(
    index: &dashmap::DashMap<String, HashSet<AgentKey>>,
    labels: &[String],
    key: &AgentKey,
) {
    for label in labels {
        index.entry(label.clone()).or_default().insert(key.clone());
    }
}

fn exact_dash(snapshot: &CatalogSnapshot, key: &AgentKey) -> AgentMetadata {
    let metadata = snapshot.entries.get(key).map(|entry| entry.value().clone());
    valid_metadata(metadata, "exact DashMap fixture entry")
}

fn exact_standard(snapshot: &ImmutableCatalogSnapshot, key: &AgentKey) -> AgentMetadata {
    valid_metadata(
        snapshot.get(key).cloned(),
        "exact standard-map fixture entry",
    )
}

fn highest_dash(snapshot: &CatalogSnapshot, name: &str) -> AgentMetadata {
    let versions = snapshot
        .name_index
        .get(name)
        .expect("DashMap fixture versions");
    let metadata = versions.iter().rev().find_map(|version| {
        snapshot
            .entries
            .get(&(name.to_owned(), version.clone()))
            .map(|entry| entry.value().clone())
            .filter(is_valid)
    });
    valid_metadata(metadata, "DashMap highest-version fixture entry")
}

fn highest_standard(snapshot: &ImmutableCatalogSnapshot, name: &str) -> AgentMetadata {
    let versions = snapshot
        .versions_for(name)
        .expect("standard-map fixture versions");
    let metadata = versions.iter().rev().find_map(|version| {
        snapshot
            .get(&(name.to_owned(), version.clone()))
            .cloned()
            .filter(is_valid)
    });
    valid_metadata(metadata, "standard-map highest-version fixture entry")
}

fn valid_metadata(metadata: Option<AgentMetadata>, message: &str) -> AgentMetadata {
    metadata.filter(is_valid).expect(message)
}

fn is_valid(metadata: &AgentMetadata) -> bool {
    matches!(&metadata.state, AgentState::Valid)
}

fn indexed_dash(snapshot: &CatalogSnapshot, tag: &str, capability: &str) -> Vec<AgentMetadata> {
    let tags = snapshot
        .tag_index
        .get(tag)
        .map(|bucket| bucket.clone())
        .expect("DashMap fixture tag index");
    let capabilities = snapshot
        .capability_index
        .get(capability)
        .map(|bucket| bucket.clone())
        .expect("DashMap fixture capability index");
    collect_valid_dash(snapshot, indexed_keys(tags, capabilities))
}

fn indexed_standard(
    snapshot: &ImmutableCatalogSnapshot,
    tag: &str,
    capability: &str,
) -> Vec<AgentMetadata> {
    let tags = snapshot
        .tagged_keys(tag)
        .cloned()
        .expect("standard-map fixture tag index");
    let capabilities = snapshot
        .capability_keys(capability)
        .cloned()
        .expect("standard-map fixture capability index");
    collect_valid_standard(snapshot, indexed_keys(tags, capabilities))
}

fn indexed_keys(tags: HashSet<AgentKey>, capabilities: HashSet<AgentKey>) -> HashSet<AgentKey> {
    tags.intersection(&capabilities).cloned().collect()
}

fn collect_valid_dash(snapshot: &CatalogSnapshot, keys: HashSet<AgentKey>) -> Vec<AgentMetadata> {
    keys.iter()
        .filter_map(|key| snapshot.entries.get(key).map(|entry| entry.value().clone()))
        .filter(|metadata| matches!(&metadata.state, AgentState::Valid))
        .collect()
}

fn collect_valid_standard(
    snapshot: &ImmutableCatalogSnapshot,
    keys: HashSet<AgentKey>,
) -> Vec<AgentMetadata> {
    keys.iter()
        .filter_map(|key| snapshot.get(key).cloned())
        .filter(|metadata| matches!(&metadata.state, AgentState::Valid))
        .collect()
}

mod catalog_representation_bench;

use catalog_representation_bench::checks::*;

criterion_group!(
    benches,
    catalog_representation_bench::bench_catalog_representations
);
criterion_main!(benches);
