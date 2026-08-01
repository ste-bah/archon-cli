//! Catalog representation benchmark for Issue #109.
//!
//! This harness compares the current `DashMap`-backed `CatalogSnapshot` with an
//! equivalent immutable representation built from standard `HashMap`s. It measures
//! complete publication plus production-equivalent exact, resolution, and indexed
//! read facades. Fixtures, `ArcSwap` targets, and equivalence checks are ready
//! before Criterion begins timing.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use arc_swap::ArcSwap;
use archon_bench::catalog_representation::{deterministic_index_checksum, metadata_digest};
use archon_core::agents::{
    AgentKey, AgentMetadata, AgentState, CatalogSnapshot, ResourceReq, SourceKind,
};
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

const FIXTURE_SIZES: [usize; 3] = [100, 1_000, 10_000];
const VERSIONS_PER_NAME: usize = 4;

#[derive(Clone, Debug, Default)]
struct StandardMapSnapshot {
    entries: HashMap<AgentKey, AgentMetadata>,
    name_index: HashMap<String, BTreeSet<semver::Version>>,
    tag_index: HashMap<String, HashSet<AgentKey>>,
    capability_index: HashMap<String, HashSet<AgentKey>>,
}

struct DashReadFacade {
    snapshot: ArcSwap<CatalogSnapshot>,
}

struct StandardReadFacade {
    snapshot: ArcSwap<StandardMapSnapshot>,
}

struct Fixture {
    dash: CatalogSnapshot,
    standard: StandardMapSnapshot,
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
            membership_index_checksum_standard(&self.standard.tag_index)
        );
        assert_eq!(
            membership_index_checksum_dash(&self.dash.capability_index),
            membership_index_checksum_standard(&self.standard.capability_index),
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
            membership_index_checksum_standard(&standard.capability_index),
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
    fn from_snapshot(snapshot: &StandardMapSnapshot) -> Self {
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

fn build_snapshots(agent_count: usize) -> (CatalogSnapshot, StandardMapSnapshot) {
    let mut dash = CatalogSnapshot::default();
    let mut standard = StandardMapSnapshot::default();
    for index in 0..agent_count {
        let metadata = fixture_metadata(index);
        let key = (metadata.name.clone(), metadata.version.clone());
        insert_dash(&mut dash, key.clone(), metadata.clone());
        insert_standard(&mut standard, key, metadata);
    }
    (dash, standard)
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

fn insert_standard(snapshot: &mut StandardMapSnapshot, key: AgentKey, metadata: AgentMetadata) {
    snapshot.entries.insert(key.clone(), metadata.clone());
    snapshot
        .name_index
        .entry(metadata.name.clone())
        .or_default()
        .insert(metadata.version.clone());
    insert_memberships_standard(&mut snapshot.tag_index, &metadata.tags, &key);
    insert_memberships_standard(&mut snapshot.capability_index, &metadata.capabilities, &key);
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

fn insert_memberships_standard(
    index: &mut HashMap<String, HashSet<AgentKey>>,
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

fn exact_standard(snapshot: &StandardMapSnapshot, key: &AgentKey) -> AgentMetadata {
    valid_metadata(
        snapshot.entries.get(key).cloned(),
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

fn highest_standard(snapshot: &StandardMapSnapshot, name: &str) -> AgentMetadata {
    let versions = snapshot
        .name_index
        .get(name)
        .expect("standard-map fixture versions");
    let metadata = versions.iter().rev().find_map(|version| {
        snapshot
            .entries
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
    snapshot: &StandardMapSnapshot,
    tag: &str,
    capability: &str,
) -> Vec<AgentMetadata> {
    let tags = snapshot
        .tag_index
        .get(tag)
        .cloned()
        .expect("standard-map fixture tag index");
    let capabilities = snapshot
        .capability_index
        .get(capability)
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
    snapshot: &StandardMapSnapshot,
    keys: HashSet<AgentKey>,
) -> Vec<AgentMetadata> {
    keys.iter()
        .filter_map(|key| snapshot.entries.get(key).cloned())
        .filter(|metadata| matches!(&metadata.state, AgentState::Valid))
        .collect()
}

fn metadata_checksum(metadata: &AgentMetadata) -> u64 {
    let digest = metadata_digest(metadata);
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("digest prefix"))
}

fn results_checksum(results: &[AgentMetadata]) -> (usize, u64) {
    let checksum = results
        .iter()
        .map(metadata_checksum)
        .fold(0_u64, u64::wrapping_add);
    (results.len(), checksum)
}

fn string_checksum(value: &str) -> u64 {
    value.bytes().fold(0_u64, |checksum, byte| {
        checksum.wrapping_mul(31).wrapping_add(byte as u64)
    })
}

fn key_checksum(key: &AgentKey) -> u64 {
    string_checksum(&key.0).wrapping_add(string_checksum(&key.1.to_string()))
}

fn entry_digests_dash(snapshot: &CatalogSnapshot) -> BTreeSet<(String, String)> {
    snapshot
        .entries
        .iter()
        .map(|entry| {
            (
                agent_key_string(entry.key()),
                metadata_digest(entry.value()).to_hex().to_string(),
            )
        })
        .collect()
}

fn entry_digests_standard(snapshot: &StandardMapSnapshot) -> BTreeSet<(String, String)> {
    snapshot
        .entries
        .iter()
        .map(|(key, metadata)| {
            (
                agent_key_string(key),
                metadata_digest(metadata).to_hex().to_string(),
            )
        })
        .collect()
}

fn snapshot_digest(snapshot: &CatalogSnapshot) -> (usize, usize, usize, usize, u64) {
    let checksum = snapshot
        .entries
        .iter()
        .map(|entry| key_checksum(entry.key()).wrapping_add(metadata_checksum(entry.value())))
        .fold(0_u64, u64::wrapping_add);
    (
        snapshot.entries.len(),
        snapshot.name_index.len(),
        snapshot.tag_index.len(),
        snapshot.capability_index.len(),
        checksum,
    )
}

fn standard_digest(snapshot: &StandardMapSnapshot) -> (usize, usize, usize, usize, u64) {
    let checksum = snapshot
        .entries
        .iter()
        .map(|(key, metadata)| key_checksum(key).wrapping_add(metadata_checksum(metadata)))
        .fold(0_u64, u64::wrapping_add);
    (
        snapshot.entries.len(),
        snapshot.name_index.len(),
        snapshot.tag_index.len(),
        snapshot.capability_index.len(),
        checksum,
    )
}

fn name_index_checksum_dash(snapshot: &CatalogSnapshot) -> u64 {
    let index = snapshot
        .name_index
        .iter()
        .map(|bucket| {
            (
                bucket.key().clone(),
                bucket
                    .value()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect();
    deterministic_index_checksum(&index)
}

fn name_index_checksum_standard(snapshot: &StandardMapSnapshot) -> u64 {
    let index = snapshot
        .name_index
        .iter()
        .map(|(name, versions)| {
            (
                name.clone(),
                versions
                    .iter()
                    .map(ToString::to_string)
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect();
    deterministic_index_checksum(&index)
}

fn membership_index_checksum_dash(index: &dashmap::DashMap<String, HashSet<AgentKey>>) -> u64 {
    let index = index
        .iter()
        .map(|bucket| {
            (
                bucket.key().clone(),
                bucket.value().iter().map(agent_key_string).collect(),
            )
        })
        .collect();
    deterministic_index_checksum(&index)
}

fn membership_index_checksum_standard(index: &HashMap<String, HashSet<AgentKey>>) -> u64 {
    let index = index
        .iter()
        .map(|(key, members)| (key.clone(), members.iter().map(agent_key_string).collect()))
        .collect();
    deterministic_index_checksum(&index)
}

fn agent_key_string(key: &AgentKey) -> String {
    format!("{}@{}", key.0, key.1)
}

fn dash_results(fixture: &Fixture) -> ResultChecks {
    let exact = metadata_checksum(&fixture.dash_reads.exact_get(&fixture.exact_key));
    let highest_version =
        metadata_checksum(&fixture.dash_reads.highest_version(&fixture.lookup_name));
    let (indexed_count, indexed_checksum) = results_checksum(
        &fixture
            .dash_reads
            .indexed_read(&fixture.tag, &fixture.capability),
    );
    ResultChecks {
        exact,
        highest_version,
        indexed_count,
        indexed_checksum,
    }
}

fn standard_results(fixture: &Fixture) -> ResultChecks {
    let exact = metadata_checksum(&fixture.standard_reads.exact_get(&fixture.exact_key));
    let highest_version =
        metadata_checksum(&fixture.standard_reads.highest_version(&fixture.lookup_name));
    let (indexed_count, indexed_checksum) = results_checksum(
        &fixture
            .standard_reads
            .indexed_read(&fixture.tag, &fixture.capability),
    );
    ResultChecks {
        exact,
        highest_version,
        indexed_count,
        indexed_checksum,
    }
}

mod catalog_representation_bench;

criterion_group!(
    benches,
    catalog_representation_bench::bench_catalog_representations
);
criterion_main!(benches);
