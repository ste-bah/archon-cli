use archon_policy::{load_policy_from_sources, EffectivePolicy, PolicySource};

fn write_policy(dir: &tempfile::TempDir, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn parses_user_facing_policy_toml() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_policy(
        &dir,
        "policy.toml",
        r#"
[policy.gametheory]
max_agents_per_council = 8
enable_tier11 = true

[policy.learning]
auto_apply_low_risk = true

[policy.docs.vlm]
enabled = true
mode = "local"
"#,
    );
    let load = load_policy_from_sources(&[PolicySource {
        label: "workspace",
        path,
    }])
    .unwrap();
    assert!(load.policy.gametheory.enable_tier11);
    assert_eq!(load.policy.gametheory.max_agents_per_council, 8);
    assert!(load.policy.learning.auto_apply_low_risk);
    assert_eq!(load.policy.docs.vlm.mode, "local");
}

#[test]
fn missing_policy_defaults_to_deny() {
    let dir = tempfile::tempdir().unwrap();
    let load = load_policy_from_sources(&[PolicySource {
        label: "workspace",
        path: dir.path().join("missing.toml"),
    }])
    .unwrap();
    assert!(load.loaded_sources.is_empty());
    assert!(!load.policy.gametheory_tier11_decision().allowed);
    assert!(!load.policy.docs_vlm_decision().allowed);
    assert!(!load.policy.learning_auto_apply_decision("RetrievalProfile", "Low").allowed);
}

#[test]
fn workspace_overrides_user_and_system_policy() {
    let dir = tempfile::tempdir().unwrap();
    let system = write_policy(&dir, "system.toml", "[policy.gametheory]\nenable_tier11 = false\n");
    let user = write_policy(&dir, "user.toml", "[policy.gametheory]\nenable_tier11 = false\nmax_cost_usd = 10.0\n");
    let workspace = write_policy(&dir, "workspace.toml", "[policy.gametheory]\nenable_tier11 = true\n");
    let load = load_policy_from_sources(&[
        PolicySource { label: "system", path: system },
        PolicySource { label: "user", path: user },
        PolicySource { label: "workspace", path: workspace },
    ])
    .unwrap();
    assert!(load.policy.gametheory.enable_tier11);
    assert_eq!(load.policy.gametheory.max_cost_usd, 10.0);
}

#[test]
fn local_vlm_requires_docs_enabled_and_worker_allow() {
    let policy = EffectivePolicy {
        docs: archon_policy::DocsPolicy {
            vlm: archon_policy::VlmPolicy {
                enabled: true,
                mode: "local".into(),
                ..Default::default()
            },
        },
        ..Default::default()
    };
    assert!(!policy.docs_vlm_decision().allowed);
    let mut allowed = policy.clone();
    allowed.workers.vlm = "allow-local".into();
    assert!(allowed.docs_vlm_decision().allowed);
}

#[test]
fn cloud_vlm_requires_dual_cloud_policy() {
    let mut policy = EffectivePolicy::default();
    policy.docs.vlm.enabled = true;
    policy.docs.vlm.mode = "cloud".into();
    policy.docs.vlm.allow_cloud = true;
    assert!(!policy.docs_vlm_decision().allowed);
    policy.network.allow_cloud_vlm = true;
    assert!(policy.docs_vlm_decision().allowed);
}

#[test]
fn tier11_gate_tracks_gametheory_policy() {
    let mut policy = EffectivePolicy::default();
    assert!(!policy.gametheory_tier11_decision().allowed);
    policy.gametheory.enable_tier11 = true;
    assert!(policy.gametheory_tier11_decision().allowed);
}

#[test]
fn high_impact_learning_changes_remain_approval_gated() {
    let mut policy = EffectivePolicy::default();
    policy.learning.auto_apply_low_risk = true;
    assert!(policy.learning_auto_apply_decision("RetrievalProfile", "Low").allowed);
    assert!(!policy.learning_auto_apply_decision("PromptProfile", "Low").allowed);
    assert!(!policy.learning_auto_apply_decision("RetrievalProfile", "High").allowed);
}
