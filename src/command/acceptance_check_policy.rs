//! Operator configuration for host verifiers, independent of observer eligibility.

#[cfg(test)]
use std::path::Path;

#[cfg(test)]
use anyhow::Context;
use anyhow::Result;
use archon_core::config::{AcceptanceExecutionConfig, ArchonConfig};
use archon_workflow::acceptance_check_environment::{CheckPolicy, validate_operator_bindings};

fn configured(policy: &AcceptanceExecutionConfig) -> Result<CheckPolicy> {
    validate_operator_bindings(
        &policy.toolchain_path,
        &policy.environment,
        &policy.environment_allowlist,
    )
    .map_err(|reason| anyhow::anyhow!("[workflow.acceptance_execution] check policy: {reason}"))?;
    Ok(CheckPolicy {
        toolchain_path: Some(policy.toolchain_path.clone()),
        bound: policy.environment.clone(),
        forwarded: policy.environment_allowlist.clone(),
    })
}

pub(crate) fn from_config(config: &ArchonConfig) -> Result<Option<CheckPolicy>> {
    config
        .workflow
        .acceptance_execution
        .as_ref()
        .map(configured)
        .transpose()
}

/// CLI uses its already-resolved config, including settings overlays.
pub(crate) fn for_config_action(
    action: &archon_workflow::CommandAction,
    config: &ArchonConfig,
) -> Result<Option<Option<CheckPolicy>>> {
    use archon_workflow::CommandAction;
    match action {
        CommandAction::Plan { .. }
        | CommandAction::Run { .. }
        | CommandAction::RunTemplate { .. } => Ok(Some(from_config(config)?)),
        _ => Ok(None),
    }
}

/// Resume/status consume the launch record, not current configuration.
#[cfg(test)]
pub(crate) fn for_new_plan(
    action: &archon_workflow::CommandAction,
    cwd: &Path,
    config_path: Option<&Path>,
) -> Result<Option<Option<CheckPolicy>>> {
    use archon_workflow::CommandAction;
    match action {
        CommandAction::Plan { .. }
        | CommandAction::Run { .. }
        | CommandAction::RunTemplate { .. } => Ok(Some(load(cwd, config_path)?)),
        _ => Ok(None),
    }
}

/// Unlike other optional workflow knobs, unreadable/malformed policy layers
/// cannot be skipped: that would silently change which data a verifier gets.
#[cfg(test)]
pub(crate) fn load(cwd: &Path, config_path: Option<&Path>) -> Result<Option<CheckPolicy>> {
    use archon_core::config_layers::{deep_merge_toml, discover_config_paths};
    let mut merged = toml::Value::Table(Default::default());
    for layer in discover_config_paths(config_path, cwd, None) {
        let text = std::fs::read_to_string(&layer.path)
            .with_context(|| format!("reading check policy at {}", layer.path.display()))?;
        let value = text
            .parse::<toml::Value>()
            .with_context(|| format!("parsing check policy at {}", layer.path.display()))?;
        merged = deep_merge_toml(merged, value);
    }
    if merged
        .get("workflow")
        .is_some_and(|value| !value.is_table())
    {
        anyhow::bail!("check policy configuration: workflow must be a table");
    }
    merged
        .get("workflow")
        .and_then(|w| w.get("acceptance_execution"))
        .map(|value| {
            let config: AcceptanceExecutionConfig = value
                .clone()
                .try_into()
                .context("[workflow.acceptance_execution] check policy")?;
            configured(&config)
        })
        .transpose()
}

#[cfg(test)]
#[path = "acceptance_check_policy_r3_tests.rs"]
mod round_three;
