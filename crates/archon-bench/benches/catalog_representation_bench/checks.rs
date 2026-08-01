use super::super::*;

pub(crate) fn metadata_checksum(metadata: &AgentMetadata) -> u64 {
    let digest = metadata_digest(metadata);
    u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("digest prefix"))
}

pub(crate) fn results_checksum(results: &[AgentMetadata]) -> (usize, u64) {
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

pub(crate) fn entry_digests_dash(snapshot: &CatalogSnapshot) -> BTreeSet<(String, String)> {
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

pub(crate) fn entry_digests_standard(snapshot: &StandardMapSnapshot) -> BTreeSet<(String, String)> {
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

pub(crate) fn snapshot_digest(snapshot: &CatalogSnapshot) -> (usize, usize, usize, usize, u64) {
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

pub(crate) fn standard_digest(snapshot: &StandardMapSnapshot) -> (usize, usize, usize, usize, u64) {
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

pub(crate) fn name_index_checksum_dash(snapshot: &CatalogSnapshot) -> u64 {
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

pub(crate) fn name_index_checksum_standard(snapshot: &StandardMapSnapshot) -> u64 {
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

pub(crate) fn membership_index_checksum_dash(
    index: &dashmap::DashMap<String, HashSet<AgentKey>>,
) -> u64 {
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

pub(crate) fn membership_index_checksum_standard(
    index: &HashMap<String, HashSet<AgentKey>>,
) -> u64 {
    let index = index
        .iter()
        .map(|(key, members)| (key.clone(), members.iter().map(agent_key_string).collect()))
        .collect();
    deterministic_index_checksum(&index)
}

fn agent_key_string(key: &AgentKey) -> String {
    format!("{}@{}", key.0, key.1)
}

pub(crate) fn dash_results(fixture: &Fixture) -> ResultChecks {
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

pub(crate) fn standard_results(fixture: &Fixture) -> ResultChecks {
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
