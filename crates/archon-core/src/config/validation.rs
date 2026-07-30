use std::path::PathBuf;

use super::*;

/// Returns the default config file path: `~/.config/archon/config.toml`
pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("archon")
        .join("config.toml")
}

/// Validate an `ArchonConfig`, returning `ConfigError::ValidationError` on
/// any invalid field values.
pub fn validate(config: &ArchonConfig) -> Result<(), ConfigError> {
    // identity.mode
    match config.identity.mode.as_str() {
        "spoof" | "clean" | "custom" => {}
        other => {
            return Err(ConfigError::ValidationError(format!(
                "identity.mode must be \"spoof\", \"clean\", or \"custom\", got \"{other}\""
            )));
        }
    }

    // permissions.mode — accepts all 6 canonical modes + legacy aliases
    if config
        .permissions
        .mode
        .parse::<archon_permissions::mode::PermissionMode>()
        .is_err()
    {
        return Err(ConfigError::ValidationError(format!(
            "permissions.mode must be a valid mode (default, acceptEdits, plan, auto, \
             dontAsk, bypassPermissions) or legacy alias (ask, yolo), got \"{}\"",
            config.permissions.mode
        )));
    }

    // tools.bash_timeout
    if config.tools.bash_timeout == 0 {
        return Err(ConfigError::ValidationError(
            "tools.bash_timeout must be > 0".into(),
        ));
    }

    // tools.max_concurrency
    if !(1..=16).contains(&config.tools.max_concurrency) {
        return Err(ConfigError::ValidationError(format!(
            "tools.max_concurrency must be 1..=16, got {}",
            config.tools.max_concurrency
        )));
    }

    if !(1..=8).contains(&config.workflow.generated.max_repair_iterations) {
        return Err(ConfigError::ValidationError(format!(
            "workflow.generated.max_repair_iterations must be 1..=8, got {}",
            config.workflow.generated.max_repair_iterations
        )));
    }
    if !(1..=8).contains(&config.workflow.generated.max_investigation_iterations) {
        return Err(ConfigError::ValidationError(format!(
            "workflow.generated.max_investigation_iterations must be 1..=8, got {}",
            config.workflow.generated.max_investigation_iterations
        )));
    }
    if !(300..=86_400).contains(&config.workflow.generated.verification_branch_timeout_secs) {
        return Err(ConfigError::ValidationError(format!(
            "workflow.generated.verification_branch_timeout_secs must be 300..=86400, got {}",
            config.workflow.generated.verification_branch_timeout_secs
        )));
    }
    if !(300..=86_400).contains(&config.workflow.generated.host_call_timeout_secs) {
        return Err(ConfigError::ValidationError(format!(
            "workflow.generated.host_call_timeout_secs must be 300..=86400, got {}",
            config.workflow.generated.host_call_timeout_secs
        )));
    }

    config
        .sandbox
        .validate()
        .map_err(ConfigError::ValidationError)?;

    // context.compact_threshold
    if !(0.0..=1.0).contains(&config.context.compact_threshold) {
        return Err(ConfigError::ValidationError(format!(
            "context.compact_threshold must be 0.0..=1.0, got {}",
            config.context.compact_threshold
        )));
    }
    if config.context.max_tool_result_bytes
        < crate::agent::tool_result_context::MIN_MAX_TOOL_RESULT_BYTES
    {
        return Err(ConfigError::ValidationError(format!(
            "context.max_tool_result_bytes must be >= {}",
            crate::agent::tool_result_context::MIN_MAX_TOOL_RESULT_BYTES,
        )));
    }

    // consciousness.energy_decay_rate
    if !(0.0..=1.0).contains(&config.consciousness.energy_decay_rate) {
        return Err(ConfigError::ValidationError(format!(
            "consciousness.energy_decay_rate must be 0.0..=1.0, got {}",
            config.consciousness.energy_decay_rate
        )));
    }
    if !(0.0..=1.0).contains(&config.consciousness.energy_regen_rate) {
        return Err(ConfigError::ValidationError(format!(
            "consciousness.energy_regen_rate must be 0.0..=1.0, got {}",
            config.consciousness.energy_regen_rate
        )));
    }
    if !(0.0..=1.0).contains(&config.consciousness.energy_floor) {
        return Err(ConfigError::ValidationError(format!(
            "consciousness.energy_floor must be 0.0..=1.0, got {}",
            config.consciousness.energy_floor
        )));
    }

    // consciousness.initial_rules
    if config.consciousness.initial_rules.len() > 50 {
        return Err(ConfigError::ValidationError(format!(
            "consciousness.initial_rules: too many rules ({}), maximum is 50",
            config.consciousness.initial_rules.len()
        )));
    }
    for (i, rule) in config.consciousness.initial_rules.iter().enumerate() {
        if rule.trim().is_empty() {
            return Err(ConfigError::ValidationError(format!(
                "consciousness.initial_rules[{i}]: rule must not be empty or whitespace-only"
            )));
        }
    }

    validate_world_model_guardrails(&config.learning.world_model.guardrails)?;
    validate_world_model_jepa(&config.learning.world_model.jepa)?;

    // personality profile
    config
        .personality
        .validate()
        .map_err(|e| ConfigError::ValidationError(e.to_string()))?;

    Ok(())
}

pub(super) fn validate_world_model_jepa(jepa: &WorldModelJepaConfig) -> Result<(), ConfigError> {
    if jepa.min_cuda_validation_examples == 0 || jepa.min_metal_validation_examples == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa min_*_validation_examples must be > 0".into(),
        ));
    }
    if !(0.0..=1.0).contains(&jepa.backend_parity_cosine_floor) {
        return Err(ConfigError::ValidationError(format!(
            "learning.world_model.jepa.backend_parity_cosine_floor must be 0.0..=1.0, got {}",
            jepa.backend_parity_cosine_floor
        )));
    }
    if jepa.max_backend_prediction_latency_ms == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.max_backend_prediction_latency_ms must be > 0".into(),
        ));
    }
    if jepa.max_backend_first_call_latency_ms == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.max_backend_first_call_latency_ms must be > 0".into(),
        ));
    }

    // T025: validate eval sub-config
    let eval = &jepa.eval;
    if !["quick", "full", "promotion"].contains(&eval.mode.as_str()) {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.eval.mode must be one of: quick, full, promotion".into(),
        ));
    }
    if eval.quick_max_runtime_ms == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.eval.quick_max_runtime_ms must be > 0 \
             (quick mode requires a bounded deadline)"
                .into(),
        ));
    }
    if eval.embedding_batch_size > eval.batch_size {
        return Err(ConfigError::ValidationError(format!(
            "learning.world_model.jepa.eval.embedding_batch_size ({}) must be <= batch_size ({})",
            eval.embedding_batch_size, eval.batch_size
        )));
    }
    if eval.eval_schema_version == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.jepa.eval.eval_schema_version must be >= 1".into(),
        ));
    }

    Ok(())
}

fn validate_world_model_guardrails(
    guardrails: &WorldModelGuardrailsConfig,
) -> Result<(), ConfigError> {
    for (name, value) in [
        ("interactive_mode", guardrails.interactive_mode.as_str()),
        ("pipeline_mode", guardrails.pipeline_mode.as_str()),
        ("tool_run_mode", guardrails.tool_run_mode.as_str()),
        (
            "verification_run_mode",
            guardrails.verification_run_mode.as_str(),
        ),
    ] {
        if !matches!(
            value,
            "off" | "learn_only" | "advisory" | "guarded" | "strict"
        ) {
            return Err(ConfigError::ValidationError(format!(
                "learning.world_model.guardrails.{name} must be off, learn_only, advisory, guarded, or strict, got \"{value}\""
            )));
        }
    }
    for (name, value) in [
        ("medium_risk_threshold", guardrails.medium_risk_threshold),
        ("high_risk_threshold", guardrails.high_risk_threshold),
        (
            "critical_risk_threshold",
            guardrails.critical_risk_threshold,
        ),
    ] {
        if !(0.0..=1.0).contains(&value) {
            return Err(ConfigError::ValidationError(format!(
                "learning.world_model.guardrails.{name} must be 0.0..=1.0, got {value}"
            )));
        }
    }
    if guardrails.medium_risk_threshold > guardrails.high_risk_threshold
        || guardrails.high_risk_threshold > guardrails.critical_risk_threshold
    {
        return Err(ConfigError::ValidationError(
            "learning.world_model.guardrails thresholds must satisfy medium <= high <= critical"
                .into(),
        ));
    }
    if guardrails.max_guardrail_overhead_ms == 0 {
        return Err(ConfigError::ValidationError(
            "learning.world_model.guardrails.max_guardrail_overhead_ms must be > 0".into(),
        ));
    }
    Ok(())
}
