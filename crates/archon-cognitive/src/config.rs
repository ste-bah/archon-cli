use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CognitiveConfig {
    pub enabled: bool,
    pub max_candidates: usize,
    pub trivial_turn_tool_policy: String,
    pub record_decisions: bool,
    pub record_reflections: bool,
    pub use_world_model: bool,
    pub use_jepa: bool,
    pub use_reasoning_quality: bool,
    pub use_self_model: bool,
    pub max_pipeline_ms: u64,
    pub situation_ttl_days: u32,
    pub reflection_ttl_days: u32,
    pub prediction_ttl_days: u32,
    pub ledger_dir: String,
}

impl Default for CognitiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_candidates: 5,
            trivial_turn_tool_policy: "none".into(),
            record_decisions: true,
            record_reflections: true,
            use_world_model: false,
            use_jepa: false,
            use_reasoning_quality: false,
            use_self_model: false,
            max_pipeline_ms: 500,
            situation_ttl_days: 90,
            reflection_ttl_days: 180,
            prediction_ttl_days: 90,
            ledger_dir: "~/.local/share/archon/cognitive".into(),
        }
    }
}

impl CognitiveConfig {
    pub fn validate_and_normalize(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        clamp_usize(
            &mut self.max_candidates,
            2,
            5,
            "learning.cognitive.max_candidates",
            &mut warnings,
        );
        if !matches!(
            self.trivial_turn_tool_policy.as_str(),
            "none" | "memory_only"
        ) {
            warnings.push(format!(
                "learning.cognitive.trivial_turn_tool_policy reset from {:?} to \"none\"",
                self.trivial_turn_tool_policy
            ));
            self.trivial_turn_tool_policy = "none".into();
        }
        clamp_u64(
            &mut self.max_pipeline_ms,
            50,
            5000,
            "learning.cognitive.max_pipeline_ms",
            &mut warnings,
        );
        warnings
    }
}

fn clamp_usize(value: &mut usize, min: usize, max: usize, name: &str, warnings: &mut Vec<String>) {
    if *value < min {
        warnings.push(format!("{name} clamped from {value} to {min}"));
        *value = min;
    } else if *value > max {
        warnings.push(format!("{name} clamped from {value} to {max}"));
        *value = max;
    }
}

fn clamp_u64(value: &mut u64, min: u64, max: u64, name: &str, warnings: &mut Vec<String>) {
    if *value < min {
        warnings.push(format!("{name} clamped from {value} to {min}"));
        *value = min;
    } else if *value > max {
        warnings.push(format!("{name} clamped from {value} to {max}"));
        *value = max;
    }
}
