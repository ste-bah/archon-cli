//! Capture native observation authority from host config, never model output.
use std::path::Path;
use archon_workflow::{WorkflowError,WorkflowResult};
use archon_workflow::acceptance_scratch::ScratchPolicy;

#[derive(Clone,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBinding {pub policy:ScratchPolicy,pub source_commit:String}

pub(crate) fn capture(project:&Path,tasks:&Path)->WorkflowResult<Option<NativeBinding>> {
    use archon_core::config_layers::{deep_merge_toml,discover_config_paths};
    let mut merged=toml::Value::Table(Default::default());
    for layer in discover_config_paths(None,project,None) {
        let text=std::fs::read_to_string(&layer.path).map_err(|source|WorkflowError::Io {path:layer.path.clone(),source})?;
        let value:toml::Value=text.parse().map_err(|e:toml::de::Error|WorkflowError::SpecInvalid(e.to_string()))?;
        merged=deep_merge_toml(merged,value);
    }
    let Some(value)=merged.get("workflow").and_then(|w|w.get("acceptance_execution")) else {return Ok(None);};
    let config:archon_core::config::AcceptanceExecutionConfig=value.clone().try_into()
        .map_err(|e:toml::de::Error|WorkflowError::SpecInvalid(e.to_string()))?;
    let canonical=|p:&Path|p.canonicalize().map_err(|source|WorkflowError::Io {path:p.to_path_buf(),source});
    let repository=canonical(&config.repository)?;
    let output=std::process::Command::new("git").arg("-C").arg(&repository).args(["rev-parse","HEAD"]).output()
        .map_err(|source|WorkflowError::Io {path:repository.clone(),source})?;
    if !output.status.success() {return Err(WorkflowError::SpecInvalid("native source repository has no recorded commit".into()));}
    let policy=ScratchPolicy {repository,project:canonical(project)?,task_root:canonical(tasks)?,
        scratch_parent:config.scratch_parent,project_inputs:config.project_inputs,
        combined:matches!(config.project_repository_view,archon_core::config::AcceptanceProjectView::Combined),
        toolchain_path:config.toolchain_path,environment:config.environment,cargo_seed:config.cargo_seed,
        timeout_secs:config.timeout_secs,output_bytes:config.output_bytes,scratch_bytes:config.scratch_bytes};
    policy.validate()?;
    Ok(Some(NativeBinding {policy,source_commit:String::from_utf8_lossy(&output.stdout).trim().into()}))
}
