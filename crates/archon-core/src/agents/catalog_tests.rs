use std::path::PathBuf;

use super::*;
use crate::agents::metadata::{AgentMetadata, AgentState, ResourceReq, SourceKind};
use chrono::Utc;

fn make_meta(name: &str, version: &str) -> AgentMetadata {
    AgentMetadata {
        name: name.to_string(),
        version: semver::Version::parse(version).unwrap(),
        description: format!("Agent {name}"),
        category: "test".to_string(),
        tags: vec!["rust".to_string()],
        capabilities: vec!["review".to_string()],
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

#[test]
fn insert_two_versions_same_name() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("foo", "1.0.0")).unwrap();
    catalog.insert(make_meta("foo", "2.0.0")).unwrap();

    assert_eq!(catalog.len(), 2);

    let snap = catalog.snapshot_immutable();
    let versions = snap.versions_for("foo").unwrap();
    assert!(versions.contains(&semver::Version::new(1, 0, 0)));
    assert!(versions.contains(&semver::Version::new(2, 0, 0)));
}

#[test]
fn insert_with_tag_populates_tag_index() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("bar", "1.0.0")).unwrap();

    let snap = catalog.snapshot_immutable();
    let tagged = snap.tagged_keys("rust").unwrap();
    assert!(tagged.contains(&("bar".to_string(), semver::Version::new(1, 0, 0))));
}

#[test]
fn insert_oversized_metadata_rejected() {
    let catalog = DiscoveryCatalog::new();
    let mut meta = make_meta("huge", "1.0.0");
    // Create a description > 10 MB
    meta.description = "x".repeat(11 * 1024 * 1024);

    let result = catalog.insert(meta);
    assert!(result.is_err());
    match result.unwrap_err() {
        DiscoveryError::MetadataTooLarge { size, .. } => {
            assert!(size > 10 * 1024 * 1024);
        }
        other => panic!("expected MetadataTooLarge, got: {other}"),
    }
}

#[test]
fn concurrent_insert_500_entries() {
    use std::sync::Arc;
    let catalog = Arc::new(DiscoveryCatalog::new());
    let mut handles = vec![];

    for thread_id in 0..10 {
        let cat = catalog.clone();
        handles.push(std::thread::spawn(move || {
            for i in 0..50 {
                let name = format!("agent-{thread_id}-{i}");
                cat.insert(make_meta(&name, "1.0.0")).unwrap();
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(catalog.len(), 500);
}

#[test]
fn snapshot_isolation_from_subsequent_insert() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("before", "1.0.0")).unwrap();

    let snap_before = catalog.snapshot_immutable();
    assert_eq!(snap_before.len(), 1);

    catalog.insert(make_meta("after", "1.0.0")).unwrap();

    // Old snapshot still shows 1 entry
    assert_eq!(snap_before.len(), 1);
    // Current catalog shows 2
    assert_eq!(catalog.len(), 2);
}

// -----------------------------------------------------------------------
// TASK-AGS-305: resolve, versions, dependencies, collision tests
// -----------------------------------------------------------------------

#[test]
fn resolve_returns_highest_version() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("foo", "1.0.0")).unwrap();
    catalog.insert(make_meta("foo", "2.0.0")).unwrap();

    let resolved = catalog.resolve("foo", None).unwrap();
    assert_eq!(resolved.version, semver::Version::new(2, 0, 0));
}

#[test]
fn resolve_with_exact_version_req() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("foo", "1.0.0")).unwrap();
    catalog.insert(make_meta("foo", "2.0.0")).unwrap();

    let req = semver::VersionReq::parse("=1.0.0").unwrap();
    let resolved = catalog.resolve("foo", Some(&req)).unwrap();
    assert_eq!(resolved.version, semver::Version::new(1, 0, 0));
}

#[test]
fn resolve_with_caret_req() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("foo", "1.0.0")).unwrap();
    catalog.insert(make_meta("foo", "1.2.0")).unwrap();
    catalog.insert(make_meta("foo", "2.0.0")).unwrap();

    let req = semver::VersionReq::parse("^1").unwrap();
    let resolved = catalog.resolve("foo", Some(&req)).unwrap();
    assert_eq!(resolved.version, semver::Version::new(1, 2, 0));
}

#[test]
fn resolve_unknown_name_suggests() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("foo", "1.0.0")).unwrap();
    catalog.insert(make_meta("bar", "1.0.0")).unwrap();

    let err = catalog.resolve("fo", None).unwrap_err();
    match err {
        DiscoveryError::AgentNotFound { suggestions, .. } => {
            assert!(
                suggestions.contains(&"foo".to_string()),
                "expected 'foo' in suggestions, got: {suggestions:?}"
            );
        }
        other => panic!("expected AgentNotFound, got: {other}"),
    }
}

#[test]
fn collision_keeps_first_entry() {
    let catalog = DiscoveryCatalog::new();
    let mut meta_a = make_meta("foo", "1.0.0");
    meta_a.source_path = PathBuf::from("/path/A");
    catalog.insert(meta_a).unwrap();

    let mut meta_b = make_meta("foo", "1.0.0");
    meta_b.source_path = PathBuf::from("/path/B");
    catalog.insert(meta_b).unwrap();

    // Only 1 entry (collision ignored)
    assert_eq!(catalog.len(), 1);
    let entry = catalog
        .get(&("foo".to_string(), semver::Version::new(1, 0, 0)))
        .unwrap();
    assert_eq!(entry.source_path, PathBuf::from("/path/A"));
}

#[test]
fn circular_dependency_detected() {
    use crate::agents::metadata::DependencyRef;
    let catalog = DiscoveryCatalog::new();

    let mut a = make_meta("agent-a", "1.0.0");
    a.dependencies = vec![DependencyRef {
        name: "agent-b".to_string(),
        version_req: semver::VersionReq::STAR,
    }];
    catalog.insert(a).unwrap();

    let mut b = make_meta("agent-b", "1.0.0");
    b.dependencies = vec![DependencyRef {
        name: "agent-a".to_string(),
        version_req: semver::VersionReq::STAR,
    }];
    catalog.insert(b).unwrap();

    let result = catalog.resolve_dependencies("agent-a");
    assert!(result.is_err());
    match result.unwrap_err() {
        DiscoveryError::CircularDependency(path) => {
            assert!(
                path.contains(&"agent-a".to_string()) && path.contains(&"agent-b".to_string()),
                "cycle path should contain both agents: {path:?}"
            );
        }
        other => panic!("expected CircularDependency, got: {other}"),
    }
}

// -----------------------------------------------------------------------
// TASK-AGS-306: filter/list tests
// -----------------------------------------------------------------------

#[test]
fn list_filter_by_tags_and_logic() {
    let catalog = DiscoveryCatalog::new();

    let mut a = make_meta("a", "1.0.0");
    a.tags = vec!["rust".into(), "refactor".into()];
    a.capabilities = vec!["review".into()];
    catalog.insert(a).unwrap();

    let mut b = make_meta("b", "1.0.0");
    b.tags = vec!["rust".into()];
    b.capabilities = vec!["run".into()];
    catalog.insert(b).unwrap();

    let mut c = make_meta("c", "1.0.0");
    c.tags = vec!["go".into()];
    c.capabilities = vec!["review".into()];
    catalog.insert(c).unwrap();

    // AND: tags=[rust] AND capabilities=[review] -> only 'a' has both
    let filter = AgentFilter {
        tags: vec!["rust".into()],
        capabilities: vec!["review".into()],
        logic: FilterLogic::And,
        ..Default::default()
    };
    let results = catalog.list(&filter);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "a");
}

#[test]
fn list_filter_or_logic() {
    let catalog = DiscoveryCatalog::new();

    let mut a = make_meta("a", "1.0.0");
    a.tags = vec!["rust".into()];
    catalog.insert(a).unwrap();

    let mut b = make_meta("b", "1.0.0");
    b.tags = vec!["go".into()];
    catalog.insert(b).unwrap();

    let filter = AgentFilter {
        tags: vec!["rust".into(), "go".into()],
        logic: FilterLogic::Or,
        ..Default::default()
    };
    let results = catalog.list(&filter);
    assert_eq!(results.len(), 2);
}

#[test]
fn list_name_pattern_filter() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("code-review", "1.0.0")).unwrap();
    catalog.insert(make_meta("code-gen", "1.0.0")).unwrap();
    catalog.insert(make_meta("test-runner", "1.0.0")).unwrap();

    let filter = AgentFilter {
        name_pattern: Some(globset::Glob::new("code-*").unwrap()),
        ..Default::default()
    };
    let results = catalog.list(&filter);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|m| m.name.starts_with("code-")));
}

#[test]
fn list_hides_invalid_by_default() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("valid", "1.0.0")).unwrap();
    let mut invalid = make_meta("invalid", "1.0.0");
    invalid.state = AgentState::Invalid("broken".into());
    catalog.insert(invalid).unwrap();

    let results = catalog.list(&AgentFilter::default());
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "valid");

    // With include_invalid
    let results = catalog.list(&AgentFilter {
        include_invalid: true,
        ..Default::default()
    });
    assert_eq!(results.len(), 2);
}

#[test]
fn list_perf_300_agents() {
    let catalog = DiscoveryCatalog::new();
    for i in 0..300 {
        let mut m = make_meta(&format!("agent-{i:03}"), "1.0.0");
        m.tags = vec!["test".into()];
        m.capabilities = vec!["run".into()];
        catalog.insert(m).unwrap();
    }

    let start = std::time::Instant::now();
    let filter = AgentFilter {
        tags: vec!["test".into()],
        capabilities: vec!["run".into()],
        logic: FilterLogic::And,
        ..Default::default()
    };
    let results = catalog.list(&filter);
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 300);
    assert!(
        elapsed.as_millis() < 100,
        "list took {}ms, expected <100ms",
        elapsed.as_millis()
    );
}

#[test]
fn resolve_skips_invalid_entries() {
    let catalog = DiscoveryCatalog::new();
    let mut invalid = make_meta("foo", "2.0.0");
    invalid.state = AgentState::Invalid("broken".to_string());
    catalog.insert(invalid).unwrap();
    catalog.insert(make_meta("foo", "1.0.0")).unwrap();

    // Should skip 2.0.0 (Invalid) and return 1.0.0 (Valid)
    let resolved = catalog.resolve("foo", None).unwrap();
    assert_eq!(resolved.version, semver::Version::new(1, 0, 0));
}

// -----------------------------------------------------------------------
// TASK-AGS-307: info() tests
// -----------------------------------------------------------------------

#[test]
fn info_returns_highest_version_by_default() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("foo", "1.0.0")).unwrap();
    catalog.insert(make_meta("foo", "2.0.0")).unwrap();

    let view = catalog.info("foo", None).unwrap();
    assert_eq!(view.selected.version, semver::Version::new(2, 0, 0));
    assert_eq!(view.all_versions.len(), 2);
    // Descending order
    assert_eq!(view.all_versions[0], semver::Version::new(2, 0, 0));
    assert_eq!(view.all_versions[1], semver::Version::new(1, 0, 0));
    assert!(view.dependency_graph.is_empty());
}

#[test]
fn info_pins_to_exact_version() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("foo", "1.0.0")).unwrap();
    catalog.insert(make_meta("foo", "2.0.0")).unwrap();

    let req = semver::VersionReq::parse("=1.0.0").unwrap();
    let view = catalog.info("foo", Some(&req)).unwrap();
    assert_eq!(view.selected.version, semver::Version::new(1, 0, 0));
}

#[test]
fn info_unknown_returns_not_found() {
    let catalog = DiscoveryCatalog::new();
    catalog.insert(make_meta("foo", "1.0.0")).unwrap();

    let result = catalog.info("unknown", None);
    assert!(result.is_err());
    match result.unwrap_err() {
        DiscoveryError::AgentNotFound { name, .. } => {
            assert_eq!(name, "unknown");
        }
        other => panic!("expected AgentNotFound, got: {other}"),
    }
}

#[test]
fn info_includes_dependency_graph() {
    use crate::agents::metadata::DependencyRef;
    let catalog = DiscoveryCatalog::new();

    let mut a = make_meta("agent-a", "1.0.0");
    a.dependencies = vec![DependencyRef {
        name: "agent-b".to_string(),
        version_req: semver::VersionReq::parse("^1").unwrap(),
    }];
    catalog.insert(a).unwrap();
    catalog.insert(make_meta("agent-b", "1.2.0")).unwrap();

    let view = catalog.info("agent-a", None).unwrap();
    assert_eq!(view.dependency_graph.len(), 1);
    assert_eq!(view.dependency_graph[0].0, "agent-b");
    assert_eq!(view.dependency_graph[0].1, semver::Version::new(1, 2, 0));
}
