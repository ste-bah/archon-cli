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
fn context_plan_model_rejects_whitespace_and_preserves_configured_value() {
    let mut cfg = ArchonConfig::default();
    cfg.context.plan_model = Some("   ".into());
    let error = validate(&cfg).expect_err("whitespace plan model must be rejected");
    assert!(error.to_string().contains("context.plan_model"));

    cfg.context.plan_model = Some("planner".into());
    validate(&cfg).expect("configured plan model must validate");
    assert_eq!(cfg.context.plan_model.as_deref(), Some("planner"));
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

#[test]
fn workflow_gate_mode_defaults_to_observe_and_parses_all_modes() {
    assert_eq!(
        ArchonConfig::default().workflow.gate_mode,
        GateMode::Observe
    );
    for (raw, expected) in [
        ("off", GateMode::Off),
        ("observe", GateMode::Observe),
        ("enforce", GateMode::Enforce),
    ] {
        let cfg: ArchonConfig = toml::from_str(&format!("[workflow]\ngate_mode = \"{raw}\"\n"))
            .expect("valid gate mode");
        assert_eq!(cfg.workflow.gate_mode, expected);
    }
    assert!(
        toml::from_str::<ArchonConfig>("[workflow]\ngate_mode = \"silent\"\n").is_err(),
        "unknown gate modes must not silently fall back"
    );
}

#[test]
fn example_config_exposes_observe_as_the_workflow_gate_default() {
    let example = write_example_config().replace("\r\n", "\n");
    let workflow = example
        .split("\n[workflow]\n")
        .nth(1)
        .expect("root workflow section")
        .split("[workflow.generated]")
        .next()
        .expect("root workflow body");
    assert!(workflow.contains("gate_mode = \"observe\""), "{workflow}");
}

#[test]
fn workflow_repository_root_is_unset_by_default_and_parses_a_path() {
    assert_eq!(ArchonConfig::default().workflow.repository_root, None);
    let cfg: ArchonConfig =
        toml::from_str("[workflow]\nrepository_root = \"../code\"\n").expect("valid path");
    assert_eq!(
        cfg.workflow.repository_root.as_deref(),
        Some(std::path::Path::new("../code"))
    );
    // The shipped template documents the key beside the other [workflow] keys.
    let example = write_example_config().replace("\r\n", "\n");
    let workflow = example
        .split("\n[workflow]\n")
        .nth(1)
        .expect("root workflow section")
        .split("\n[workflow.")
        .next()
        .expect("root workflow body");
    assert!(workflow.contains("# repository_root = "), "{workflow}");
}

/// Issue-225: the residual pass ceiling is 1..=50, and the default is valid.
#[test]
fn max_residual_passes_is_validated_to_one_through_fifty() {
    let mut cfg = ArchonConfig::default();
    assert!(validate(&cfg).is_ok());
    for bad in [0, 51] {
        cfg.workflow.generated.max_residual_passes = bad;
        let error = validate(&cfg).unwrap_err().to_string();
        assert!(error.contains("max_residual_passes"), "{error}");
    }
    cfg.workflow.generated.max_residual_passes = 1;
    assert!(validate(&cfg).is_ok());
}

/// Issue 282: an allowlisted acceptance variable may only carry data. A name a
/// loader, toolchain, interpreter, shell or git reads to change what runs is
/// refused when the configuration is loaded, by name and with the reason.
fn acceptance_allowlist(names: &[&str]) -> String {
    format!(
        "[workflow.acceptance_execution]\nrepository=\"/repo\"\nscratch_parent=\"/scratch\"\n\
         project_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin\"\n\
         environment_allowlist={names:?}\ntimeout_secs=60\noutput_bytes=8192\nscratch_bytes=1024\n"
    )
}

#[test]
fn acceptance_allowlist_refuses_execution_controls_at_load() {
    for name in [
        "LD_PRELOAD",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER",
        "PYTHONSTARTUP",
        "NODE_OPTIONS",
        "BASH_ENV",
        "GIT_SSH_COMMAND",
        "ld_preload",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, acceptance_allowlist(&[name])).unwrap();
        let error = load_config_if_exists(path).expect_err(name).to_string();
        assert!(
            error.contains("workflow.acceptance_execution.environment_allowlist"),
            "{name}: {error}"
        );
        assert!(error.contains(&format!("'{name}'")), "{name}: {error}");
    }
}

#[test]
fn acceptance_allowlist_accepts_provider_data_names() {
    let cfg: ArchonConfig = toml::from_str(&acceptance_allowlist(&[
        "POLYGON_API_KEY",
        "OPENBB_API_URL",
    ]))
    .unwrap();
    validate(&cfg).expect("provider data names are allowed");
}
