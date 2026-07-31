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
    let filter = archon_core::agents::catalog::AgentFilter {
        tags: vec![tag.into()],
        ..Default::default()
    };
    catalog
        .list(&filter)
        .into_iter()
        .map(|agent| agent.name)
        .collect()
}

fn names_for_capability(catalog: &DiscoveryCatalog, capability: &str) -> Vec<String> {
    let filter = archon_core::agents::catalog::AgentFilter {
        capabilities: vec![capability.into()],
        ..Default::default()
    };
    catalog
        .list(&filter)
        .into_iter()
        .map(|agent| agent.name)
        .collect()
}

// Regression coverage for #107 unresolved dependency reporting.
#[test]
fn missing_exact_dependency_reports_parent_and_requirement() {
    let catalog = DiscoveryCatalog::new();
    let mut root = metadata("root", "1.0.0");
    root.dependencies = vec![DependencyRef {
        name: "missing".into(),
        version_req: semver::VersionReq::parse("=2.3.4").unwrap(),
    }];
    catalog.insert(root).expect("insert root");

    assert!(matches!(
        catalog.resolve_dependencies("root"),
        Err(DiscoveryError::UnresolvedDependency(details))
            if details.required_by == ("root".into(), "1.0.0".parse().unwrap())
                && details.name == "missing"
                && details.version_req == "=2.3.4".parse().unwrap()
    ));
}

#[test]
fn mismatched_dependency_requirement_is_unresolved() {
    let catalog = DiscoveryCatalog::new();
    let mut root = metadata("root", "1.0.0");
    root.dependencies = vec![DependencyRef {
        name: "child".into(),
        version_req: semver::VersionReq::parse("^2").unwrap(),
    }];
    catalog.insert(root).expect("insert root");
    catalog
        .insert(metadata("child", "1.0.0"))
        .expect("insert child");

    assert!(matches!(
        catalog.resolve_dependencies("root"),
        Err(DiscoveryError::UnresolvedDependency(details))
            if details.name == "child" && details.version_req == "^2".parse().unwrap()
    ));
}

#[test]
fn transitive_missing_dependency_reports_immediate_parent() {
    let catalog = DiscoveryCatalog::new();
    let mut root = metadata("root", "1.0.0");
    root.dependencies = vec![DependencyRef {
        name: "middle".into(),
        version_req: semver::VersionReq::STAR,
    }];
    let mut middle = metadata("middle", "1.2.0");
    middle.dependencies = vec![DependencyRef {
        name: "missing".into(),
        version_req: semver::VersionReq::STAR,
    }];
    catalog.insert(root).expect("insert root");
    catalog.insert(middle).expect("insert middle");

    assert!(matches!(
        catalog.resolve_dependencies("root"),
        Err(DiscoveryError::UnresolvedDependency(details))
            if details.required_by == ("middle".into(), "1.2.0".parse().unwrap())
                && details.name == "missing"
    ));
}

#[test]
fn absent_dependency_root_remains_not_found() {
    let catalog = DiscoveryCatalog::new();

    assert!(matches!(
        catalog.resolve_dependencies("absent"),
        Err(DiscoveryError::AgentNotFound { name, .. }) if name == "absent"
    ));
}

#[test]
fn constrained_intermediate_uses_selected_version_dependencies() {
    let catalog = DiscoveryCatalog::new();
    let mut root = metadata("root", "1.0.0");
    root.dependencies = vec![DependencyRef {
        name: "middle".into(),
        version_req: semver::VersionReq::parse("=1.0.0").unwrap(),
    }];
    let mut middle_v1 = metadata("middle", "1.0.0");
    middle_v1.dependencies = vec![DependencyRef {
        name: "leaf".into(),
        version_req: semver::VersionReq::STAR,
    }];
    let mut middle_v2 = metadata("middle", "2.0.0");
    middle_v2.dependencies = vec![DependencyRef {
        name: "missing-from-latest".into(),
        version_req: semver::VersionReq::STAR,
    }];
    catalog.insert(root).expect("insert root");
    catalog.insert(middle_v1).expect("insert middle v1");
    catalog.insert(middle_v2).expect("insert middle v2");
    catalog
        .insert(metadata("leaf", "1.0.0"))
        .expect("insert leaf");

    assert_eq!(
        catalog.resolve_dependencies("root").expect("resolve root"),
        vec![
            ("leaf".into(), "1.0.0".parse().unwrap()),
            ("middle".into(), "1.0.0".parse().unwrap()),
        ]
    );
}

#[test]
fn info_uses_selected_root_version_and_propagates_dependency_errors() {
    let catalog = DiscoveryCatalog::new();
    let mut root_v1 = metadata("root", "1.0.0");
    root_v1.dependencies = vec![DependencyRef {
        name: "leaf".into(),
        version_req: semver::VersionReq::STAR,
    }];
    let mut root_v2 = metadata("root", "2.0.0");
    root_v2.dependencies = vec![DependencyRef {
        name: "missing".into(),
        version_req: semver::VersionReq::STAR,
    }];
    catalog.insert(root_v1).expect("insert root v1");
    catalog.insert(root_v2).expect("insert root v2");
    catalog
        .insert(metadata("leaf", "1.0.0"))
        .expect("insert leaf");

    let v1 = semver::VersionReq::parse("=1.0.0").unwrap();
    assert_eq!(
        catalog
            .info("root", Some(&v1))
            .expect("info root v1")
            .dependency_graph,
        vec![("leaf".into(), "1.0.0".parse().unwrap())]
    );

    let v2 = semver::VersionReq::parse("=2.0.0").unwrap();
    assert!(matches!(
        catalog.info("root", Some(&v2)),
        Err(DiscoveryError::UnresolvedDependency(details))
            if details.required_by == ("root".into(), "2.0.0".parse().unwrap())
                && details.name == "missing"
    ));
}

#[test]
fn shared_dependency_is_deduplicated_in_deterministic_post_order() {
    let catalog = DiscoveryCatalog::new();
    let mut root = metadata("root", "1.0.0");
    root.dependencies = vec![
        DependencyRef {
            name: "left".into(),
            version_req: semver::VersionReq::STAR,
        },
        DependencyRef {
            name: "right".into(),
            version_req: semver::VersionReq::STAR,
        },
    ];
    let mut left = metadata("left", "1.0.0");
    left.dependencies = vec![DependencyRef {
        name: "shared".into(),
        version_req: semver::VersionReq::STAR,
    }];
    let mut right = metadata("right", "1.0.0");
    right.dependencies = left.dependencies.clone();
    for agent in [root, left, right, metadata("shared", "1.0.0")] {
        catalog.insert(agent).expect("insert DAG agent");
    }

    assert_eq!(
        catalog.resolve_dependencies("root").expect("resolve DAG"),
        vec![
            ("shared".into(), "1.0.0".parse().unwrap()),
            ("left".into(), "1.0.0".parse().unwrap()),
            ("right".into(), "1.0.0".parse().unwrap()),
        ]
    );
}

#[test]
fn same_name_different_versions_do_not_form_a_false_cycle() {
    let catalog = DiscoveryCatalog::new();
    let mut a_v1 = metadata("a", "1.0.0");
    a_v1.dependencies = vec![DependencyRef {
        name: "b".into(),
        version_req: "=1.0.0".parse().unwrap(),
    }];
    let mut b_v1 = metadata("b", "1.0.0");
    b_v1.dependencies = vec![DependencyRef {
        name: "a".into(),
        version_req: "=2.0.0".parse().unwrap(),
    }];
    for agent in [a_v1, b_v1, metadata("a", "2.0.0")] {
        catalog.insert(agent).expect("insert versioned agent");
    }

    let a_v1_req = "=1.0.0".parse().unwrap();
    assert_eq!(
        catalog
            .info("a", Some(&a_v1_req))
            .expect("resolve acyclic version path")
            .dependency_graph,
        vec![
            ("a".into(), "2.0.0".parse().unwrap()),
            ("b".into(), "1.0.0".parse().unwrap()),
        ]
    );
}

// Regression coverage for #108: same-source replacement must reconcile indexes.
#[test]
fn replacement_reconciles_stale_indexes() {
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

    assert!(names(&catalog, "old-tag").is_empty());
    assert!(names_for_capability(&catalog, "old-capability").is_empty());
    assert_eq!(names(&catalog, "new-tag"), ["replacement-agent"]);
    assert_eq!(
        names_for_capability(&catalog, "new-capability"),
        ["replacement-agent"]
    );
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
