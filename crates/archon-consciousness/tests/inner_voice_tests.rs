use archon_consciousness::inner_voice::{InnerVoice, InnerVoiceSnapshot};

#[test]
fn new_defaults() {
    let iv = InnerVoice::new();
    assert!((iv.confidence - 0.7).abs() < f32::EPSILON);
    assert!((iv.energy - 1.0).abs() < f32::EPSILON);
    assert!(iv.focus.is_empty());
    assert!(iv.struggles.is_empty());
    assert!(iv.successes.is_empty());
    assert_eq!(iv.turn_count, 0);
    assert_eq!(iv.corrections_received, 0);
}

#[test]
fn tool_success_increases_confidence() {
    let mut iv = InnerVoice::new();
    iv.on_tool_success("Read");
    assert!(iv.confidence > 0.7);
}

#[test]
fn tool_success_regenerates_energy() {
    let mut iv = InnerVoice::new();
    iv.energy = 0.5;
    iv.on_tool_success("Read");
    assert!((iv.energy - 0.505).abs() < 0.0001);
}

#[test]
fn energy_regeneration_rate_is_configurable() {
    let mut iv = InnerVoice::with_energy_policy(0.98, 0.2, 0.1);
    iv.energy = 0.5;
    iv.on_tool_success("Read");
    assert!((iv.energy - 0.7).abs() < 0.0001);
}

#[test]
fn tool_success_energy_capped_at_one() {
    let mut iv = InnerVoice::with_energy_policy(0.98, 0.2, 0.1);
    iv.energy = 0.95;
    iv.on_tool_success("Read");
    assert!((iv.energy - 1.0).abs() < f32::EPSILON);
}

#[test]
fn tool_failure_decreases_confidence() {
    let mut iv = InnerVoice::new();
    iv.on_tool_failure("Bash");
    assert!(iv.confidence < 0.7);
}

#[test]
fn three_failures_adds_struggle() {
    let mut iv = InnerVoice::new();
    iv.on_tool_failure("bash");
    iv.on_tool_failure("bash");
    iv.on_tool_failure("bash");
    assert!(iv.struggles.contains(&"bash".to_string()));
}

#[test]
fn user_correction_decreases_confidence() {
    let mut iv = InnerVoice::new();
    iv.on_user_correction();
    assert!(iv.confidence < 0.7);
}

#[test]
fn energy_decays_over_turns() {
    let mut iv = InnerVoice::new();
    for _ in 0..10 {
        iv.on_turn_complete();
    }
    assert!(iv.energy < 1.0);
    assert!(iv.energy >= 0.1);
}

#[test]
fn energy_decay_respects_floor() {
    let mut iv = InnerVoice::new();
    for _ in 0..300 {
        iv.on_turn_complete();
    }
    assert!((iv.energy - 0.1).abs() < 0.0001);
}

#[test]
fn confidence_capped_at_one() {
    let mut iv = InnerVoice::new();
    for _ in 0..100 {
        iv.on_tool_success("Read");
    }
    assert!(iv.confidence <= 1.0);
}

#[test]
fn confidence_floored_at_zero() {
    let mut iv = InnerVoice::new();
    for _ in 0..100 {
        iv.on_tool_failure("Bash");
    }
    assert!(iv.confidence >= 0.0);
}

#[test]
fn success_tracked() {
    let mut iv = InnerVoice::new();
    iv.on_tool_success("Read");
    assert!(iv.successes.contains(&"Read".to_string()));
}

#[test]
fn focus_updates_on_action() {
    let mut iv = InnerVoice::new();
    iv.on_tool_success("Edit");
    assert!(iv.focus.contains("Edit"));
}

#[test]
fn prompt_block_format() {
    let iv = InnerVoice::new();
    let block = iv.to_prompt_block();
    assert!(block.contains("<inner_voice>"));
    assert!(block.contains("</inner_voice>"));
}

#[test]
fn prompt_block_contains_fields() {
    let mut iv = InnerVoice::new();
    iv.on_tool_success("Read");
    iv.on_tool_failure("Bash");
    iv.on_tool_failure("Bash");
    iv.on_tool_failure("Bash");
    iv.on_user_correction();
    iv.on_turn_complete();

    let block = iv.to_prompt_block();
    assert!(block.contains("Confidence:"));
    assert!(block.contains("Energy:"));
    assert!(block.contains("Focus:"));
    assert!(block.contains("Struggles:"));
    assert!(block.contains("Successes:"));
    assert!(block.contains("Turns:"));
    assert!(block.contains("Corrections:"));
}

#[test]
fn snapshot_roundtrip() {
    let mut iv = InnerVoice::new();
    iv.on_tool_success("Read");
    iv.on_tool_failure("Bash");
    iv.on_tool_failure("Bash");
    iv.on_tool_failure("Bash");
    iv.on_user_correction();
    iv.on_turn_complete();

    let snapshot = iv.on_compaction();
    let restored = InnerVoice::from_snapshot(snapshot);

    assert!((iv.confidence - restored.confidence).abs() < f32::EPSILON);
    assert!((restored.energy - 1.0).abs() < f32::EPSILON);
    assert!(restored.focus.is_empty());
    assert_eq!(iv.struggles, restored.struggles);
    assert_eq!(iv.successes, restored.successes);
    assert_eq!(iv.turn_count, restored.turn_count);
    assert_eq!(iv.corrections_received, restored.corrections_received);
}

#[test]
fn snapshot_restore_resets_session_capacity_fields() {
    let snapshot = InnerVoiceSnapshot {
        confidence: 0.42,
        energy: 0.05,
        focus: "stale previous task".to_string(),
        struggles: vec!["Bash".to_string()],
        successes: vec!["Read".to_string()],
        turn_count: 145,
        corrections_received: 3,
    };

    let restored = InnerVoice::from_snapshot(snapshot);

    assert!((restored.confidence - 0.42).abs() < f32::EPSILON);
    assert!((restored.energy - 1.0).abs() < f32::EPSILON);
    assert!(restored.focus.is_empty());
    assert_eq!(restored.struggles, vec!["Bash".to_string()]);
    assert_eq!(restored.successes, vec!["Read".to_string()]);
    assert_eq!(restored.turn_count, 145);
    assert_eq!(restored.corrections_received, 3);
}

#[test]
fn snapshot_serializes_to_json() {
    let iv = InnerVoice::new();
    let snapshot = iv.on_compaction();
    let json = serde_json::to_string(&snapshot);
    assert!(json.is_ok());
}

#[test]
fn disabled_when_config_false() {
    assert!(!InnerVoice::is_enabled(false));
    assert!(InnerVoice::is_enabled(true));
}
