use archon_policy::{CognitivePolicy, PolicySource, load_policy_from_sources};

#[test]
fn default_policy_is_fail_closed_except_safe_suppression() {
    let policy = CognitivePolicy::default();
    assert!(!policy.enabled);
    assert!(!policy.allow_autonomous_tick);
    assert!(!policy.allow_background_daemon);
    assert!(policy.allow_tool_suppression);
    assert!(!policy.allow_jepa_action_scoring);
    assert!(!policy.allow_self_model_updates);
    assert!(!policy.allow_autonomous_low_risk_apply);
    assert_eq!(policy.max_autonomous_risk, "Low");
    assert!(policy.require_human_for_prompt_changes);
    assert!(policy.require_human_for_policy_changes);
    assert!(policy.require_human_for_network_changes);
    assert!(policy.require_human_for_blocking_gate_changes);
    assert!(!policy.store_raw_turn_text);
}

#[test]
fn empty_toml_yields_defaults() {
    let policy: CognitivePolicy = toml::from_str("").expect("empty policy");
    assert_eq!(policy, CognitivePolicy::default());
}

#[test]
fn validation_accepts_only_low_or_medium_risk() {
    let mut policy = CognitivePolicy::default();
    assert!(policy.validate().is_ok());
    policy.max_autonomous_risk = "Medium".into();
    assert!(policy.validate().is_ok());
    policy.max_autonomous_risk = "High".into();
    assert!(policy.validate().is_err());
    policy.max_autonomous_risk = "Critical".into();
    assert!(policy.validate().is_err());
}

#[test]
fn convenience_methods_respect_master_switch() {
    let mut policy = CognitivePolicy::default();
    assert!(policy.is_passthrough());
    assert!(!policy.can_suppress_tools());
    assert!(!policy.can_use_jepa());
    assert!(!policy.can_update_self_model());
    assert!(!policy.can_auto_apply());
    assert!(!policy.can_run_daemon());

    policy.enabled = true;
    policy.allow_autonomous_tick = true;
    policy.allow_background_daemon = true;
    policy.allow_jepa_action_scoring = true;
    policy.allow_self_model_updates = true;
    policy.allow_autonomous_low_risk_apply = true;
    assert!(policy.can_suppress_tools());
    assert!(policy.can_use_jepa());
    assert!(policy.can_update_self_model());
    assert!(policy.can_auto_apply());
    assert!(policy.can_run_daemon());
    assert!(policy.prompt_changes_require_human());
    assert!(policy.policy_changes_require_human());
    assert!(policy.network_changes_require_human());
    assert!(policy.blocking_gate_changes_require_human());
}

#[test]
fn effective_policy_missing_cognitive_section_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("policy.toml");
    std::fs::write(&path, "[policy.web]\nallow_mutating_actions = true\n").unwrap();
    let load = load_policy_from_sources(&[PolicySource {
        label: "workspace",
        path,
    }])
    .expect("load policy");
    assert_eq!(load.policy.cognitive, CognitivePolicy::default());
    assert!(load.policy.web.allow_mutating_actions);
}

#[test]
fn effective_policy_loads_cognitive_section() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("policy.toml");
    std::fs::write(
        &path,
        r#"
[policy.cognitive]
enabled = true
allow_autonomous_tick = true
allow_background_daemon = true
allow_tool_suppression = false
allow_jepa_action_scoring = true
allow_self_model_updates = true
allow_autonomous_low_risk_apply = false
max_autonomous_risk = "Medium"
require_human_for_prompt_changes = true
require_human_for_policy_changes = true
require_human_for_network_changes = true
require_human_for_blocking_gate_changes = true
store_raw_turn_text = true
"#,
    )
    .unwrap();
    let load = load_policy_from_sources(&[PolicySource {
        label: "workspace",
        path,
    }])
    .expect("load policy");
    let policy = load.policy.cognitive;
    assert!(policy.enabled);
    assert!(policy.can_run_daemon());
    assert!(!policy.can_suppress_tools());
    assert!(policy.can_use_jepa());
    assert!(policy.can_update_self_model());
    assert!(!policy.can_auto_apply());
    assert_eq!(policy.max_autonomous_risk, "Medium");
    assert!(policy.store_raw_turn_text);
}

#[test]
fn full_policy_round_trip() {
    let policy: CognitivePolicy = toml::from_str(
        r#"
enabled = true
allow_autonomous_tick = true
allow_background_daemon = true
allow_tool_suppression = true
allow_jepa_action_scoring = true
allow_self_model_updates = true
allow_autonomous_low_risk_apply = true
max_autonomous_risk = "Medium"
require_human_for_prompt_changes = true
require_human_for_policy_changes = true
require_human_for_network_changes = true
require_human_for_blocking_gate_changes = true
store_raw_turn_text = false
"#,
    )
    .expect("parse policy");
    let encoded = toml::to_string(&policy).expect("serialize policy");
    let decoded: CognitivePolicy = toml::from_str(encoded.as_str()).expect("decode policy");
    assert_eq!(decoded, policy);
    assert!(decoded.enabled);
    assert!(decoded.can_run_daemon());
    assert_eq!(decoded.max_autonomous_risk, "Medium");
    assert!(decoded.validate().is_ok());
}
