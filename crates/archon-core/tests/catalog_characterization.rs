use std::path::PathBuf;

use archon_core::agents::{
    AgentMetadata, AgentState, DependencyRef, DiscoveryCatalog, DiscoveryError, ResourceReq,
    SourceKind,
};
use chrono::Utc;

fn metadata(name: &str, version: &str) -> AgentMetadata {
    AgentMetadata {
        name: name.into(),
        version: version.parse().expect("valid test version"),
        description: format!("{name} description"),
        category: "test".into(),
        tags: vec!["current".into()],
        capabilities: vec!["current-capability".into()],
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
    let mut filter = archon_core::agents::catalog::AgentFilter::default();
    filter.tags = vec![tag.into()];
    catalog
        .list(&filter)
        .into_iter()
        .map(|agent| agent.name)
        .collect()
}

fn names_for_capability(catalog: &DiscoveryCatalog, capability: &str) -> Vec<String> {
    let mut filter = archon_core::agents::catalog::AgentFilter::default();
    filter.capabilities = vec![capability.into()];
    catalog
        .list(&filter)
        .into_iter()
        .map(|agent| agent.name)
        .collect()
}

// Characterizes quirk tracked in #107; do not fix in #91.
#[test]
fn unresolved_dependencies_are_currently_omitted() {
    let catalog = DiscoveryCatalog::new();
    let mut root = metadata("root", "1.0.0");
    root.dependencies = vec![DependencyRef {
        name: "missing".into(),
        version_req: semver::VersionReq::STAR,
    }];
    catalog.insert(root).expect("insert root");

    assert_eq!(
        catalog.resolve_dependencies("root").expect("resolve root"),
        Vec::<(String, semver::Version)>::new()
    );
}

// Characterizes quirk tracked in #108; do not fix in #91.
#[test]
fn replacement_preserves_current_stale_index_behavior() {
    let catalog = DiscoveryCatalog::new();
    let mut original = metadata("replacement-agent", "1.0.0");
    original.tags = vec!["old-tag".into()];
    original.capabilities = vec!["old-capability".into()];
    catalog.insert(original).expect("insert original");

    let mut replacement = metadata("replacement-agent", "1.0.0");
    replacement.tags = vec!["new-tag".into()];
    replacement.capabilities = vec!["new-capability".into()];
    catalog
        .insert(replacement)
        .expect("replace same-path entry");

    let mut old_tag_names = names(&catalog, "old-tag");
    // DashMap iteration order is non-contractual; sort before comparing.
    old_tag_names.sort();
    assert_eq!(old_tag_names, ["replacement-agent"]);

    let mut old_capability_names = names_for_capability(&catalog, "old-capability");
    // DashMap iteration order is non-contractual; sort before comparing.
    old_capability_names.sort();
    assert_eq!(old_capability_names, ["replacement-agent"]);
}

#[test]
fn versions_listing_source_and_errors_are_deterministic() {
    let catalog = DiscoveryCatalog::new();
    let mut oldest = metadata("ordered", "1.0.0");
    oldest.source_kind = SourceKind::Remote;
    catalog.insert(oldest).expect("insert oldest");
    catalog
        .insert(metadata("ordered", "2.0.0"))
        .expect("insert newest");
    catalog
        .insert(metadata("another", "1.0.0"))
        .expect("insert another");

    assert_eq!(
        catalog.versions("ordered"),
        vec!["2.0.0".parse().unwrap(), "1.0.0".parse().unwrap()]
    );
    assert_eq!(
        catalog.resolve("ordered", None).unwrap().source_kind,
        SourceKind::Local
    );
    assert_eq!(
        catalog
            .list(&archon_core::agents::catalog::AgentFilter::default())
            .into_iter()
            .map(|agent| (agent.name, agent.version))
            .collect::<Vec<_>>(),
        vec![
            ("another".into(), "1.0.0".parse().unwrap()),
            ("ordered".into(), "2.0.0".parse().unwrap()),
            ("ordered".into(), "1.0.0".parse().unwrap()),
        ]
    );
    assert!(matches!(
        catalog.resolve("absent", None),
        Err(DiscoveryError::AgentNotFound { name, .. }) if name == "absent"
    ));
}
