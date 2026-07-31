use std::{path::PathBuf, sync::Arc};

use archon_core::agents::{
    AgentMetadata, AgentState, DiscoveryCatalog, DiscoveryError, ResourceReq, SourceKind,
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
    let snapshot = catalog.snapshot();
    for bucket in snapshot.tag_index.iter() {
        assert!(!bucket.is_empty());
        assert!(bucket.iter().all(|key| {
            snapshot
                .entries
                .get(key)
                .is_some_and(|entry| entry.tags.contains(bucket.key()))
        }));
    }
    for bucket in snapshot.capability_index.iter() {
        assert!(!bucket.is_empty());
        assert!(bucket.iter().all(|key| {
            snapshot
                .entries
                .get(key)
                .is_some_and(|entry| entry.capabilities.contains(bucket.key()))
        }));
    }
    for entry in snapshot.entries.iter() {
        assert!(
            snapshot
                .name_index
                .get(&entry.name)
                .is_some_and(|versions| versions.contains(&entry.version))
        );
        assert!(entry.tags.iter().all(|tag| {
            snapshot
                .tag_index
                .get(tag)
                .is_some_and(|keys| keys.contains(entry.key()))
        }));
        assert!(entry.capabilities.iter().all(|capability| {
            snapshot
                .capability_index
                .get(capability)
                .is_some_and(|keys| keys.contains(entry.key()))
        }));
    }
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
            .snapshot()
            .entries
            .contains_key(&("immediate".into(), "1.0.0".parse().unwrap()))
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
    let snapshot = catalog.snapshot();
    assert!(!snapshot.tag_index.contains_key("obsolete-tag"));
    assert!(
        !snapshot
            .capability_index
            .contains_key("obsolete-capability")
    );
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
        DiscoveryError::DuplicateAgent { ref existing_path, ref rejected_path, .. }
            if existing_path == &PathBuf::from("/agents/winner")
                && rejected_path == &contender_path
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
