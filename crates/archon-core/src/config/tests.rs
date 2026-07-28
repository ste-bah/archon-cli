use super::validation::validate_world_model_jepa;
use super::*;

#[test]
fn initial_rules_default_is_empty() {
    assert!(ConsciousnessConfig::default().initial_rules.is_empty());
}

#[test]
fn initial_rules_deserialized_from_toml() {
    let toml_str = r#"
            [consciousness]
            initial_rules = ["rule a", "rule b"]
        "#;
    let cfg: ArchonConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(cfg.consciousness.initial_rules.len(), 2);
    assert_eq!(cfg.consciousness.initial_rules[0], "rule a");
}

#[test]
fn initial_rules_empty_string_rejected() {
    let mut cfg = ArchonConfig::default();
    cfg.consciousness.initial_rules = vec!["".to_string()];
    assert!(validate(&cfg).is_err());
}

#[test]
fn initial_rules_whitespace_only_rejected() {
    let mut cfg = ArchonConfig::default();
    cfg.consciousness.initial_rules = vec!["   ".to_string()];
    let err = validate(&cfg).unwrap_err();
    assert!(err.to_string().contains("whitespace"));
}

#[test]
fn initial_rules_max_50_enforced() {
    let mut cfg = ArchonConfig::default();
    cfg.consciousness.initial_rules = (0..51).map(|i| format!("rule {i}")).collect();
    assert!(validate(&cfg).is_err());

    cfg.consciousness.initial_rules = (0..50).map(|i| format!("rule {i}")).collect();
    assert!(validate(&cfg).is_ok());
}

#[test]
fn write_example_config_is_valid_toml() {
    let s = write_example_config();
    let cfg: ArchonConfig = toml::from_str(&s).expect("should parse as ArchonConfig");
    validate(&cfg).expect("should validate");
}

#[test]
fn write_example_config_contains_personality_section() {
    assert!(write_example_config().contains("[personality]"));
}

#[test]
fn write_example_config_contains_consciousness_section() {
    assert!(write_example_config().contains("[consciousness]"));
}

#[test]
fn write_example_config_contains_world_model_guardrails_section() {
    assert!(write_example_config().contains("[learning.world_model.guardrails]"));
    let cfg: ArchonConfig = toml::from_str(&write_example_config()).unwrap();
    assert_eq!(
        cfg.learning.world_model.guardrails.interactive_mode,
        "advisory"
    );
    assert_eq!(cfg.learning.world_model.guardrails.pipeline_mode, "guarded");
    assert_eq!(
        cfg.learning
            .world_model
            .guardrails
            .max_guardrail_overhead_ms,
        40
    );
}

#[test]
fn world_model_guardrail_config_validation_rejects_bad_modes_and_thresholds() {
    let mut cfg = ArchonConfig::default();
    cfg.learning.world_model.guardrails.interactive_mode = "YOLO".into();
    assert!(validate(&cfg).is_err());

    let mut cfg = ArchonConfig::default();
    cfg.learning.world_model.guardrails.medium_risk_threshold = 0.80;
    cfg.learning.world_model.guardrails.high_risk_threshold = 0.70;
    assert!(validate(&cfg).is_err());

    let mut cfg = ArchonConfig::default();
    cfg.learning
        .world_model
        .guardrails
        .max_guardrail_overhead_ms = 0;
    assert!(validate(&cfg).is_err());
}

#[test]
fn write_example_config_contains_initial_rules() {
    assert!(write_example_config().contains("initial_rules"));
}

#[test]
fn write_example_config_personality_fields_round_trip() {
    let s = write_example_config();
    let cfg: ArchonConfig = toml::from_str(&s).unwrap();
    assert_eq!(cfg.personality.name, "Archon");
    assert_eq!(cfg.personality.mbti_type, "INTJ");
    assert_eq!(cfg.personality.enneagram, "4w5");
    assert!(!cfg.personality.traits.is_empty());
}

#[test]
fn write_example_config_initial_rules_non_empty() {
    let s = write_example_config();
    let cfg: ArchonConfig = toml::from_str(&s).unwrap();
    assert!(!cfg.consciousness.initial_rules.is_empty());
}

#[test]
fn ssh_agent_forwarding_defaults_to_false() {
    let cfg = ArchonConfig::default();
    assert!(!cfg.remote.ssh.agent_forwarding);
}

#[test]
fn ssh_agent_forwarding_true_deserialized() {
    let toml_str = r#"
            [remote.ssh]
            agent_forwarding = true
        "#;
    let cfg: ArchonConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.remote.ssh.agent_forwarding);
}

#[test]
fn ssh_agent_forwarding_false_deserialized() {
    let toml_str = r#"
            [remote.ssh]
            agent_forwarding = false
        "#;
    let cfg: ArchonConfig = toml::from_str(toml_str).unwrap();
    assert!(!cfg.remote.ssh.agent_forwarding);
}

#[test]
fn ssh_agent_forwarding_absent_defaults_false() {
    let toml_str = r#"
            [remote.ssh]
            port = 2222
        "#;
    let cfg: ArchonConfig = toml::from_str(toml_str).unwrap();
    assert!(!cfg.remote.ssh.agent_forwarding);
}

// -------------------------------------------------------------------------
// T025: WorldModelJepaEvalConfig validation tests
// -------------------------------------------------------------------------

#[test]
fn validate_jepa_eval_rejects_invalid_mode() {
    let mut config = WorldModelJepaConfig::default();
    config.eval.mode = "invalid".to_string();
    let result = validate_world_model_jepa(&config);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("mode"));
}

#[test]
fn validate_jepa_eval_accepts_quick_full_promotion() {
    for valid_mode in &["quick", "full", "promotion"] {
        let mut config = WorldModelJepaConfig::default();
        config.eval.mode = valid_mode.to_string();
        assert!(
            validate_world_model_jepa(&config).is_ok(),
            "{valid_mode} must be valid"
        );
    }
}

#[test]
fn validate_jepa_eval_rejects_zero_quick_runtime() {
    let mut config = WorldModelJepaConfig::default();
    config.eval.quick_max_runtime_ms = 0;
    let result = validate_world_model_jepa(&config);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("quick_max_runtime_ms")
    );
}

#[test]
fn validate_jepa_eval_rejects_oversized_embedding_batch() {
    let mut config = WorldModelJepaConfig::default();
    config.eval.batch_size = 64;
    config.eval.embedding_batch_size = 256; // > batch_size
    let result = validate_world_model_jepa(&config);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("embedding_batch_size")
    );
}

#[test]
fn validate_jepa_eval_rejects_zero_schema_version() {
    let mut config = WorldModelJepaConfig::default();
    config.eval.eval_schema_version = 0;
    let result = validate_world_model_jepa(&config);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("eval_schema_version")
    );
}

#[test]
fn default_jepa_eval_config_passes_validation() {
    let config = WorldModelJepaConfig::default();
    assert!(validate_world_model_jepa(&config).is_ok());
}
