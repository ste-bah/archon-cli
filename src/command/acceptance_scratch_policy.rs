//! Capture native observation authority from host config, never model output.
use archon_workflow::acceptance_scratch::ScratchPolicy;
use archon_workflow::{WorkflowError, WorkflowResult};
use std::path::Path;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeBinding {
    pub policy: ScratchPolicy,
    pub source_commit: String,
}

pub(crate) fn capture(project: &Path, tasks: &Path) -> WorkflowResult<Option<NativeBinding>> {
    use archon_core::config_layers::{deep_merge_toml, discover_config_paths};
    let mut merged = toml::Value::Table(Default::default());
    for layer in discover_config_paths(None, project, None) {
        let text = std::fs::read_to_string(&layer.path).map_err(|source| WorkflowError::Io {
            path: layer.path.clone(),
            source,
        })?;
        let value: toml::Value = text
            .parse()
            .map_err(|e: toml::de::Error| WorkflowError::SpecInvalid(e.to_string()))?;
        merged = deep_merge_toml(merged, value);
    }
    let Some(value) = merged
        .get("workflow")
        .and_then(|w| w.get("acceptance_execution"))
    else {
        return Ok(None);
    };
    let config: archon_core::config::AcceptanceExecutionConfig = value
        .clone()
        .try_into()
        .map_err(|e: toml::de::Error| WorkflowError::SpecInvalid(e.to_string()))?;
    let canonical = |p: &Path| {
        p.canonicalize().map_err(|source| WorkflowError::Io {
            path: p.to_path_buf(),
            source,
        })
    };
    let repository = canonical(&config.repository)?;
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(&repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|source| WorkflowError::Io {
            path: repository.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(WorkflowError::SpecInvalid(
            "native source repository has no recorded commit".into(),
        ));
    }
    let policy = ScratchPolicy {
        repository,
        project: canonical(project)?,
        task_root: canonical(tasks)?,
        scratch_parent: config.scratch_parent,
        project_inputs: config.project_inputs,
        combined: matches!(
            config.project_repository_view,
            archon_core::config::AcceptanceProjectView::Combined
        ),
        toolchain_path: config.toolchain_path,
        environment: config.environment,
        cargo_seed: config.cargo_seed,
        timeout_secs: config.timeout_secs,
        output_bytes: config.output_bytes,
        scratch_bytes: config.scratch_bytes,
    };
    policy.validate()?;
    Ok(Some(NativeBinding {
        policy,
        source_commit: String::from_utf8_lossy(&output.stdout).trim().into(),
    }))
}

/// Called by the finalizer before its first terminal record. Once recorded,
/// recovery uses this commit even if the checkout has subsequently moved.
pub(crate) fn record_final_source(
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
    binding: &serde_json::Value,
) -> WorkflowResult<serde_json::Value> {
    let repo = binding
        .get("policy")
        .and_then(|p| p.get("repository"))
        .and_then(|r| r.as_str())
        .ok_or_else(|| {
            WorkflowError::StateCorrupt("native policy has no source repository".into())
        })?;
    let repo = Path::new(repo)
        .canonicalize()
        .map_err(|source| WorkflowError::Io {
            path: repo.into(),
            source,
        })?;
    let path = store.run_dir(run_id).join("observer/source-revision.json");
    let record = if path.exists() {
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).map_err(|source| WorkflowError::Io {
                path: path.clone(),
                source,
            })?)?;
        if value["repository"].as_str() != repo.to_str() || value["run_id"].as_str() != Some(run_id)
        {
            return Err(WorkflowError::StateCorrupt(
                "native final source record identity differs".into(),
            ));
        }
        value
    } else {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .map_err(|source| WorkflowError::Io {
                path: repo.clone(),
                source,
            })?;
        if !output.status.success() {
            return Err(WorkflowError::StateCorrupt(
                "cannot record final implementation commit".into(),
            ));
        }
        let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if commit.len() != 40 || !commit.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(WorkflowError::StateCorrupt(
                "invalid final source commit".into(),
            ));
        }
        let value = serde_json::json!({"run_id":run_id,"repository":repo,"commit":commit});
        store.write_run_json(run_id, "observer/source-revision.json", &value)?;
        value
    };
    let mut updated = binding.clone();
    updated["source_commit"] = record["commit"].clone();
    Ok(updated)
}
