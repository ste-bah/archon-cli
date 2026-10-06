//! Read the operator's launch policy, never verifier/task-authored input.

use std::path::Path;

use super::CheckPolicy;
use crate::acceptance_scratch::ScratchPolicy;

pub const RUN_POLICY_REMEDY: &str = "Name the needed variables in [workflow.acceptance_execution] environment_allowlist in the operator configuration and relaunch the workflow to record check_environment_policy in v2/generated-metadata.json";

pub const NO_RUN_POLICY_REMEDY: &str = "Supply this runner with the operator-owned run_root whose v2/generated-metadata.json records [workflow.acceptance_execution] environment_allowlist; without a run binding this runner consumes only the default check policy";

/// The workflow's recorded `[workflow.acceptance_execution]` policy.
/// An absent snapshot/section means no policy. An unreadable, corrupt or
/// invalid recorded binding is an operational error, never a default policy.
pub fn policy_for_run(run_root: Option<&Path>) -> Result<Option<CheckPolicy>, String> {
    let Some(root) = run_root else {
        return Ok(None);
    };
    let path = root.join("v2/generated-metadata.json");
    let refused = |reason: String| {
        format!(
            "check command policy at {} could not be read: {reason}; repair the operator configuration and relaunch the workflow",
            path.display()
        )
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(refused(error.to_string())),
    };
    let metadata: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| refused(error.to_string()))?;
    if !metadata.is_object() {
        return Err(refused("launch metadata must be an object".into()));
    }
    // New launches record this independently of run-end observer eligibility.
    // An explicit null records no configured policy, and does not select a
    // different authority from a legacy observer binding.
    if let Some(value) = metadata.get("check_environment_policy") {
        if value.is_null() {
            return Ok(None);
        }
        let policy: CheckPolicy =
            serde_json::from_value(value.clone()).map_err(|error| refused(error.to_string()))?;
        validate_operator_bindings(
            policy
                .toolchain_path
                .as_deref()
                .ok_or_else(|| refused("configured check policy has no toolchain PATH".into()))?,
            &policy.bound,
            &policy.forwarded,
        )
        .map_err(refused)?;
        return Ok(Some(policy));
    }
    if let Some(snapshot) = metadata
        .get("observer_snapshot")
        .filter(|value| !value.is_null())
        && !snapshot.is_object()
    {
        return Err(refused("observer snapshot must be an object".into()));
    }
    let Some(binding) = metadata
        .pointer("/observer_snapshot/native_execution")
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    if let Some(error) = binding.get("capture_error") {
        return Err(refused(format!("operator policy capture failed: {error}")));
    }
    let policy: ScratchPolicy = serde_json::from_value(
        binding
            .get("policy")
            .cloned()
            .ok_or_else(|| refused("native execution binding has no policy".into()))?,
    )
    .map_err(|error| refused(error.to_string()))?;
    policy
        .validate()
        .map_err(|error| refused(error.to_string()))?;
    Ok(Some(CheckPolicy::configured(&policy)))
}

/// The same configured environment rules for scratch and host verifiers.
/// Static bindings contain only reviewed nonsecret knobs; data comes from
/// the host via the allowlist, never from a recorded policy's literal values.
pub fn validate_operator_bindings(
    path: &str,
    environment: &std::collections::BTreeMap<String, String>,
    allowlist: &[String],
) -> Result<(), String> {
    if std::env::split_paths(path).any(|p| p.as_os_str().is_empty() || !p.is_absolute()) {
        return Err("toolchain PATH must contain absolute directories".into());
    }
    for key in allowlist {
        archon_shell::data_environment::check_data_variable(key)
            .map_err(|reason| format!("acceptance environment allowlist: {reason}"))?;
    }
    for (key, value) in environment {
        if !matches!(
            key.as_str(),
            "LANG" | "LC_ALL" | "TZ" | "RUSTUP_HOME" | "RUSTUP_TOOLCHAIN"
        ) || value.contains('\0')
        {
            return Err(format!(
                "environment key '{key}' is not a permitted nonsecret scratch binding"
            ));
        }
    }
    Ok(())
}
