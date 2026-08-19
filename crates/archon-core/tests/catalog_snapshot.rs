use std::{path::PathBuf, sync::Arc};

use archon_core::agents::catalog::ImmutableCatalogSnapshot as CatalogImmutableCatalogSnapshot;
use archon_core::agents::{
    AgentMetadata, AgentState, DiscoveryCatalog, DiscoveryError, ImmutableCatalogSnapshot,
    ResourceReq, SourceKind,
};
use chrono::Utc;

fn metadata(name: &str) -> AgentMetadata {
    AgentMetadata {
        name: name.into(),
        version: "1.0.0".parse().unwrap(),
        description: name.into(),
        category: "test".into(),
        tags: vec!["current".into()],
        capabilities: vec!["current".into()],
        input_schema: serde_json::json!({}),
        output_schema: serde_json::json!({}),
        resource_requirements: ResourceReq::default(),
        dependencies: vec![],
        source_path: PathBuf::from(format!("/agents/{name}")),
        source_kind: SourceKind::Local,
        state: AgentState::Valid,
        loaded_at: Utc::now(),
    }
}

fn names(catalog: &DiscoveryCatalog, tag: &str) -> Vec<String> {
    catalog
        .list(&archon_core::agents::catalog::AgentFilter {
            tags: vec![tag.into()],
            ..Default::default()
        })
        .into_iter()
        .map(|agent| agent.name)
        .collect()
}

fn names_for_capability(catalog: &DiscoveryCatalog, capability: &str) -> Vec<String> {
    catalog
        .list(&archon_core::agents::catalog::AgentFilter {
            capabilities: vec![capability.into()],
            ..Default::default()
        })
        .into_iter()
        .map(|agent| agent.name)
        .collect()
}

fn assert_snapshot_invariant(catalog: &DiscoveryCatalog) {
    assert_immutable_snapshot_invariant(&catalog.snapshot_immutable());
}

fn assert_immutable_snapshot_invariant(snapshot: &ImmutableCatalogSnapshot) {
    for (tag, keys) in snapshot.tag_index() {
        assert!(!keys.is_empty());
        assert!(keys.iter().all(|key| {
            snapshot
                .get(key)
                .is_some_and(|entry| entry.tags.contains(tag))
        }));
    }
    for (capability, keys) in snapshot.capability_index() {
        assert!(!keys.is_empty());
        assert!(keys.iter().all(|key| {
            snapshot
                .get(key)
                .is_some_and(|entry| entry.capabilities.contains(capability))
        }));
    }
    for (key, entry) in snapshot.entries() {
        assert!(
            snapshot
                .versions_for(&entry.name)
                .is_some_and(|versions| versions.contains(&entry.version))
        );
        assert!(entry.tags.iter().all(|tag| {
            snapshot
                .tagged_keys(tag)
                .is_some_and(|keys| keys.contains(key))
        }));
        assert!(entry.capabilities.iter().all(|capability| {
            snapshot
                .capability_keys(capability)
                .is_some_and(|keys| keys.contains(key))
        }));
    }
}
#[test]
fn immutable_snapshot_is_exported_from_catalog_and_agents_paths() {
    let catalog = DiscoveryCatalog::new();
    let _: Arc<ImmutableCatalogSnapshot> = catalog.snapshot_immutable();
    let _: Arc<CatalogImmutableCatalogSnapshot> = catalog.snapshot_immutable();
}
#[test]
fn immutable_snapshot_read_your_writes_and_old_snapshot_isolation() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(metadata("before")).unwrap();
    let old = catalog.snapshot_immutable();

    catalog.insert_all(vec![metadata("after"), metadata("also-after")]);
    let current = catalog.snapshot_immutable();

    assert_eq!(old.len(), 1);
    assert!(
        old.get(&("before".into(), "1.0.0".parse().unwrap()))
            .is_some()
    );
    assert!(
        old.get(&("after".into(), "1.0.0".parse().unwrap()))
            .is_none()
    );
    assert_eq!(current.len(), 3);
    assert_immutable_snapshot_invariant(&current);
}

#[test]
#[allow(deprecated)]
fn legacy_snapshot_conversion_is_equal_and_mutation_isolated() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(metadata("legacy")).unwrap();
    let immutable = catalog.snapshot_immutable();
    let legacy = catalog.snapshot();
    let key = ("legacy".into(), "1.0.0".parse().unwrap());

    assert_eq!(immutable.len(), legacy.entries.len());
    assert_eq!(
        immutable.get(&key).map(|metadata| &metadata.name),
        legacy
            .entries
            .get(&key)
            .as_deref()
            .map(|metadata| &metadata.name)
    );
    legacy.entries.remove(&key);

    assert!(catalog.snapshot_immutable().get(&key).is_some());
    assert!(immutable.get(&key).is_some());
}

#[test]
fn immutable_snapshot_accessors_expose_complete_indexes() {
    let catalog = DiscoveryCatalog::new();
    let mut entry = metadata("indexed");
    entry.tags = vec!["tag-a".into(), "tag-b".into()];
    entry.capabilities = vec!["capability-a".into()];
    catalog.insert(entry).unwrap();

    let snapshot = catalog.snapshot_immutable();
    assert_eq!(snapshot.entries().count(), 1);
    assert_eq!(snapshot.name_index().count(), 1);
    assert_eq!(snapshot.tag_index().count(), 2);
    assert_eq!(snapshot.capability_index().count(), 1);
    assert_immutable_snapshot_invariant(&snapshot);
}

#[test]
fn immutable_published_snapshots_remain_consistent_with_concurrent_writers() {
    let catalog = Arc::new(DiscoveryCatalog::new());
    let writers = (0..4)
        .map(|writer| {
            let catalog = catalog.clone();
            std::thread::spawn(move || {
                for entry in 0..25 {
                    catalog
                        .insert(metadata(&format!("{writer}-{entry}")))
                        .unwrap();
                }
            })
        })
        .collect::<Vec<_>>();
    let readers = (0..4)
        .map(|_| {
            let catalog = catalog.clone();
            std::thread::spawn(move || {
                for _ in 0..100 {
                    assert_immutable_snapshot_invariant(&catalog.snapshot_immutable());
                }
            })
        })
        .collect::<Vec<_>>();

    for writer in writers {
        writer.join().unwrap();
    }
    for reader in readers {
        reader.join().unwrap();
    }
    assert_eq!(catalog.snapshot_immutable().len(), 100);
}

#[test]
fn repeated_replacement_removes_every_prior_membership() {
    let catalog = DiscoveryCatalog::new();
    for tag in ["first", "second", "third"] {
        let mut entry = metadata("replaced");
        entry.tags = vec![tag.into()];
        entry.capabilities = vec![format!("{tag}-capability")];
        catalog.insert(entry).unwrap();
    }
    assert!(names(&catalog, "first").is_empty());
    assert!(names(&catalog, "second").is_empty());
    assert!(names_for_capability(&catalog, "first-capability").is_empty());
    assert_snapshot_invariant(&catalog);
}

#[test]
fn different_path_collision_preserves_first_entry_without_contender_indexes() {
    let catalog = DiscoveryCatalog::new();
    let mut first = metadata("collision");
    first.tags = vec!["winner".into()];
    first.source_path = "/agents/winner".into();
    catalog.insert(first).unwrap();
    let mut contender = metadata("collision");
    contender.tags = vec!["rejected".into()];
    contender.source_path = "/agents/contender".into();
    catalog.insert(contender).unwrap();
    assert_eq!(names(&catalog, "winner"), ["collision"]);
    assert!(names(&catalog, "rejected").is_empty());
    assert_eq!(
        catalog
            .get(&("collision".into(), "1.0.0".parse().unwrap()))
            .unwrap()
            .source_path,
        PathBuf::from("/agents/winner")
    );
    assert_snapshot_invariant(&catalog);
}

#[test]
fn concurrent_distinct_path_collision_keeps_consistent_winner_indexes() {
    for _round in 0..8 {
        let catalog = Arc::new(DiscoveryCatalog::new());
        let barrier = Arc::new(std::sync::Barrier::new(16));
        let handles = (0..16)
            .map(|id| {
                let catalog = catalog.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut entry = metadata("concurrent-collision");
                    entry.tags = vec![format!("tag-{id}")];
                    entry.capabilities = vec![format!("capability-{id}")];
                    entry.source_path = format!("/agents/{id}").into();
                    barrier.wait();
                    catalog.insert(entry).unwrap();
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        let winner = catalog
            .get(&("concurrent-collision".into(), "1.0.0".parse().unwrap()))
            .unwrap();
        assert_eq!(names(&catalog, &winner.tags[0]), ["concurrent-collision"]);
        assert_eq!(
            names_for_capability(&catalog, &winner.capabilities[0]),
            ["concurrent-collision"]
        );
        assert_snapshot_invariant(&catalog);
    }
}

#[test]
fn concurrent_same_path_replacements_match_final_primary_metadata() {
    let catalog = Arc::new(DiscoveryCatalog::new());
    let handles = (0..16)
        .map(|id| {
            let catalog = catalog.clone();
            std::thread::spawn(move || {
                let mut entry = metadata("concurrent-replacement");
                entry.tags = vec![format!("tag-{id}")];
                entry.capabilities = vec![format!("capability-{id}")];
                catalog.insert(entry).unwrap();
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    let entry = catalog
        .get(&("concurrent-replacement".into(), "1.0.0".parse().unwrap()))
        .unwrap();
    assert_eq!(names(&catalog, &entry.tags[0]), ["concurrent-replacement"]);
    assert_eq!(
        names_for_capability(&catalog, &entry.capabilities[0]),
        ["concurrent-replacement"]
    );
    assert_snapshot_invariant(&catalog);
}

#[test]
fn completed_insert_is_visible_in_immediate_snapshot() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(metadata("immediate")).unwrap();
    assert!(
        catalog
            .snapshot_immutable()
            .get(&("immediate".into(), "1.0.0".parse().unwrap()))
            .is_some()
    );
}

#[test]
fn obsolete_secondary_buckets_are_removed() {
    let catalog = DiscoveryCatalog::new();
    let mut original = metadata("buckets");
    original.tags = vec!["obsolete-tag".into()];
    original.capabilities = vec!["obsolete-capability".into()];
    catalog.insert(original).unwrap();
    let mut replacement = metadata("buckets");
    replacement.tags = vec!["current-tag".into()];
    replacement.capabilities = vec!["current-capability".into()];
    catalog.insert(replacement).unwrap();
    let snapshot = catalog.snapshot_immutable();
    assert!(snapshot.tagged_keys("obsolete-tag").is_none());
    assert!(snapshot.capability_keys("obsolete-capability").is_none());
}

#[test]
fn bulk_insert_rejects_invalid_metadata_without_publishing_it() {
    let catalog = DiscoveryCatalog::new();
    let accepted = metadata("accepted");
    let mut rejected = metadata("rejected");
    rejected.description = "x".repeat(11 * 1024 * 1024);
    let rejected_path = rejected.source_path.clone();

    let result = catalog.insert_all(vec![accepted, rejected]);

    assert_eq!(result.accepted.loaded, 1);
    assert_eq!(result.accepted.invalid, 0);
    assert_eq!(result.rejected.len(), 1);
    assert_eq!(result.rejected[0].metadata.name, "rejected");
    assert_eq!(result.rejected[0].metadata.source_path, rejected_path);
    assert!(matches!(
        result.rejected[0].error,
        DiscoveryError::MetadataTooLarge { ref path, .. } if path == &rejected_path
    ));
    assert_eq!(catalog.len(), 1);
}

#[test]
fn bulk_insert_excludes_rejected_invalid_metadata_from_state_counts() {
    let catalog = DiscoveryCatalog::new();
    let mut invalid = metadata("invalid");
    invalid.state = AgentState::Invalid("missing required field".into());
    invalid.description = "x".repeat(11 * 1024 * 1024);

    let result = catalog.insert_all(vec![invalid]);

    assert_eq!(result.accepted.loaded, 0);
    assert_eq!(result.accepted.invalid, 0);
    assert_eq!(result.rejected.len(), 1);
    assert_eq!(catalog.len(), 0);
}

#[test]
fn bulk_insert_rejects_same_key_contender_with_its_metadata_and_error() {
    let catalog = DiscoveryCatalog::new();
    let mut winner = metadata("collision");
    winner.source_path = "/agents/winner".into();
    catalog.insert(winner).unwrap();

    let mut contender = metadata("collision");
    contender.source_path = "/agents/contender".into();
    let contender_path = contender.source_path.clone();
    let result = catalog.insert_all(vec![contender]);

    assert_eq!(result.accepted.loaded, 0);
    assert_eq!(result.rejected.len(), 1);
    assert_eq!(result.rejected[0].metadata.source_path, contender_path);
    assert!(matches!(
        result.rejected[0].error,
        DiscoveryError::DuplicateAgent(ref duplicate)
            if duplicate.existing_path == std::path::Path::new("/agents/winner")
                && duplicate.rejected_path == contender_path
    ));
    assert_eq!(
        catalog
            .get(&("collision".into(), "1.0.0".parse().unwrap()))
            .unwrap()
            .source_path,
        PathBuf::from("/agents/winner")
    );
}

#[test]
fn bulk_insert_publishes_all_entries_and_reconciles_replacements() {
    let catalog = DiscoveryCatalog::new();
    let mut original = metadata("bulk-replaced");
    original.tags = vec!["old".into()];
    let mut replacement = metadata("bulk-replaced");
    replacement.tags = vec!["new".into()];
    let result = catalog.insert_all(vec![original, replacement, metadata("bulk-new")]);
    assert_eq!(result.accepted.loaded, 3);
    assert_eq!(result.accepted.invalid, 0);
    assert!(result.rejected.is_empty());
    assert_eq!(catalog.len(), 2);
    assert!(names(&catalog, "old").is_empty());
    assert_eq!(names(&catalog, "new"), ["bulk-replaced"]);
    assert_snapshot_invariant(&catalog);
}
