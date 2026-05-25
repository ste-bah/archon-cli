use archon_cognitive::CognitiveConfig;

#[test]
fn config_defaults_fail_closed() {
    let config = CognitiveConfig::default();
    assert!(!config.enabled);
    assert!(!config.use_world_model);
    assert!(!config.use_jepa);
    assert!(!config.use_reasoning_quality);
    assert!(!config.use_self_model);
    assert_eq!(config.max_candidates, 5);
    assert_eq!(config.trivial_turn_tool_policy, "none");
}

#[test]
fn empty_toml_yields_defaults() {
    let config: CognitiveConfig = toml::from_str("").expect("empty config");
    assert_eq!(config, CognitiveConfig::default());
}

#[test]
fn validation_clamps_bounds_and_resets_bad_policy() {
    let mut low = CognitiveConfig {
        max_candidates: 1,
        max_pipeline_ms: 10,
        trivial_turn_tool_policy: "all".into(),
        ..Default::default()
    };
    let warnings = low.validate_and_normalize();
    assert_eq!(low.max_candidates, 2);
    assert_eq!(low.max_pipeline_ms, 50);
    assert_eq!(low.trivial_turn_tool_policy, "none");
    assert_eq!(warnings.len(), 3);

    let mut high = CognitiveConfig {
        max_candidates: 100,
        max_pipeline_ms: 10000,
        trivial_turn_tool_policy: "memory_only".into(),
        ..Default::default()
    };
    let warnings = high.validate_and_normalize();
    assert_eq!(high.max_candidates, 5);
    assert_eq!(high.max_pipeline_ms, 5000);
    assert_eq!(high.trivial_turn_tool_policy, "memory_only");
    assert_eq!(warnings.len(), 2);
}

#[test]
fn config_round_trips_through_toml() {
    let input = r#"
enabled = true
max_candidates = 3
trivial_turn_tool_policy = "memory_only"
record_decisions = true
record_reflections = false
use_world_model = true
use_jepa = true
use_reasoning_quality = true
use_self_model = true
max_pipeline_ms = 750
situation_ttl_days = 30
reflection_ttl_days = 60
prediction_ttl_days = 45
ledger_dir = "/tmp/archon-cognitive"
"#;
    let config: CognitiveConfig = toml::from_str(input).expect("parse config");
    let encoded = toml::to_string(&config).expect("serialize config");
    let decoded: CognitiveConfig = toml::from_str(encoded.as_str()).expect("decode config");
    assert_eq!(decoded, config);
}
