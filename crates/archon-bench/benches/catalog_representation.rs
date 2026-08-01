//! Catalog representation benchmark for Issue #109.
//!
//! This harness compares the current `DashMap`-backed `CatalogSnapshot` with an
//! equivalent immutable representation built from standard `HashMap`s. It
//! measures complete representation deep clone/preparation plus `ArcSwap::store`
//! (the production publication boundary), exact lookup, highest-version lookup,
//! and `FilterLogic::And` candidate construction from one tag and one capability.
//! Fixtures are deterministic and built before Criterion starts timing.

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

/// An immutable standard-library-map representation equivalent to the current
/// catalog snapshot fields. It is benchmark-only and intentionally not part of
/// the production catalog API.
#[derive(Clone, Debug, Default)]
struct StandardMapSnapshot {
    entries: HashMap<AgentKey, AgentMetadata>,
    name_index: HashMap<String, BTreeSet<semver::Version>>,
    tag_index: HashMap<String, std::collections::HashSet<AgentKey>>,
    capability_index: HashMap<String, std::collections::HashSet<AgentKey>>,
}

struct Fixture {
    dash: CatalogSnapshot,
    standard: StandardMapSnapshot,
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

        let mut dash = CatalogSnapshot::default();
        let mut standard = StandardMapSnapshot::default();
        for index in 0..agent_count {
            let metadata = fixture_metadata(index);
            let key = (metadata.name.clone(), metadata.version.clone());
            insert_dash(&mut dash, key.clone(), metadata.clone());
            insert_standard(&mut standard, key, metadata);
        }

        let representative = agent_count / 2 / VERSIONS_PER_NAME;
        let fixture = Self {
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
            name_index_checksum_standard(&self.standard),
            "name index checksum"
        );
        assert_eq!(
            membership_index_checksum_dash(&self.dash.tag_index),
            membership_index_checksum_standard(&self.standard.tag_index),
            "tag index checksum"
        );
        assert_eq!(dash_results(self), standard_results(self));
    }

    fn complete_clone_matches_standard(&self) {
        assert_eq!(
            entry_digests_dash(&self.dash.clone()),
            entry_digests_standard(&self.standard.clone())
        );
    }
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
    for tag in &metadata.tags {
        snapshot
            .tag_index
            .entry(tag.clone())
            .or_default()
            .insert(key.clone());
    }
    for capability in &metadata.capabilities {
        snapshot
            .capability_index
            .entry(capability.clone())
            .or_default()
            .insert(key.clone());
    }
}

fn insert_standard(snapshot: &mut StandardMapSnapshot, key: AgentKey, metadata: AgentMetadata) {
    snapshot.entries.insert(key.clone(), metadata.clone());
    snapshot
        .name_index
        .entry(metadata.name.clone())
        .or_default()
        .insert(metadata.version.clone());
    for tag in &metadata.tags {
        snapshot
            .tag_index
            .entry(tag.clone())
            .or_default()
            .insert(key.clone());
    }
    for capability in &metadata.capabilities {
        snapshot
            .capability_index
            .entry(capability.clone())
            .or_default()
            .insert(key.clone());
    }
}

fn metadata_checksum(metadata: &AgentMetadata) -> u64 {
    let digest = metadata_digest(metadata);
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("digest prefix"))
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
    let exact = fixture
        .dash
        .entries
        .get(&fixture.exact_key)
        .map(|entry| metadata_checksum(entry.value()))
        .expect("exact DashMap fixture entry");
    let highest_version = highest_dash(&fixture.dash, &fixture.lookup_name);
    let (indexed_count, indexed_checksum) =
        indexed_dash(&fixture.dash, &fixture.tag, &fixture.capability);
    ResultChecks {
        exact,
        highest_version,
        indexed_count,
        indexed_checksum,
    }
}

fn standard_results(fixture: &Fixture) -> ResultChecks {
    let exact = fixture
        .standard
        .entries
        .get(&fixture.exact_key)
        .map(metadata_checksum)
        .expect("exact standard-map fixture entry");
    let highest_version = highest_standard(&fixture.standard, &fixture.lookup_name);
    let (indexed_count, indexed_checksum) =
        indexed_standard(&fixture.standard, &fixture.tag, &fixture.capability);
    ResultChecks {
        exact,
        highest_version,
        indexed_count,
        indexed_checksum,
    }
}

fn highest_dash(snapshot: &CatalogSnapshot, name: &str) -> u64 {
    let version = snapshot
        .name_index
        .get(name)
        .and_then(|versions| versions.iter().next_back().cloned())
        .expect("DashMap fixture versions");
    snapshot
        .entries
        .get(&(name.to_owned(), version))
        .map(|entry| metadata_checksum(entry.value()))
        .expect("DashMap highest-version fixture entry")
}

fn highest_standard(snapshot: &StandardMapSnapshot, name: &str) -> u64 {
    let version = snapshot
        .name_index
        .get(name)
        .and_then(|versions| versions.iter().next_back().cloned())
        .expect("standard-map fixture versions");
    snapshot
        .entries
        .get(&(name.to_owned(), version))
        .map(metadata_checksum)
        .expect("standard-map highest-version fixture entry")
}

fn indexed_dash(snapshot: &CatalogSnapshot, tag: &str, capability: &str) -> (usize, u64) {
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
    indexed_candidates(tags, capabilities)
}

fn indexed_standard(snapshot: &StandardMapSnapshot, tag: &str, capability: &str) -> (usize, u64) {
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
    indexed_candidates(tags, capabilities)
}

fn indexed_candidates(tags: HashSet<AgentKey>, capabilities: HashSet<AgentKey>) -> (usize, u64) {
    let matching: HashSet<_> = tags.intersection(&capabilities).cloned().collect();
    let checksum = matching
        .iter()
        .map(key_checksum)
        .fold(0_u64, u64::wrapping_add);
    (matching.len(), checksum)
}

mod catalog_representation_bench;

criterion_group!(
    benches,
    catalog_representation_bench::bench_catalog_representations
);
criterion_main!(benches);
