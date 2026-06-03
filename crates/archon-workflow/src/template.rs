use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::spec::WorkflowSpec;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedWorkflowTemplate {
    pub name: String,
    pub spec: WorkflowSpec,
    pub sanitized: bool,
}

#[derive(Debug, Clone)]
pub struct TemplateRegistry {
    root: PathBuf,
}

impl TemplateRegistry {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn project(project_root: impl AsRef<Path>) -> Self {
        Self::new(
            project_root
                .as_ref()
                .join(".archon")
                .join("workflow-templates"),
        )
    }

    pub fn save(&self, name: &str, spec: &WorkflowSpec) -> WorkflowResult<SavedWorkflowTemplate> {
        let template = SavedWorkflowTemplate {
            name: sanitize_name(name)?,
            spec: sanitize_spec(spec)?,
            sanitized: true,
        };
        fs::create_dir_all(&self.root).map_err(|e| WorkflowError::io(&self.root, e))?;
        let path = self.root.join(format!("{}.yaml", template.name));
        let yaml = serde_yaml_ng::to_string(&template)?;
        fs::write(&path, yaml).map_err(|e| WorkflowError::io(path, e))?;
        Ok(template)
    }

    pub fn load(&self, name: &str) -> WorkflowResult<SavedWorkflowTemplate> {
        let safe = sanitize_name(name)?;
        let path = self.root.join(format!("{safe}.yaml"));
        let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
        Ok(serde_yaml_ng::from_str(&raw)?)
    }
}

pub fn sanitize_spec(spec: &WorkflowSpec) -> WorkflowResult<WorkflowSpec> {
    let mut sanitized = spec.clone();
    sanitized.permissions.clear();
    sanitized.quality_gates.remove("run_id");
    for stage in &mut sanitized.stages {
        stage.model = None;
        stage.provider = None;
    }
    sanitized.validate()?;
    let yaml = sanitized.to_yaml()?;
    reject_secret_shapes(&yaml)?;
    Ok(sanitized)
}

fn sanitize_name(name: &str) -> WorkflowResult<String> {
    let safe: String = name
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if safe.is_empty() || safe.contains("..") {
        return Err(WorkflowError::UnsafeTemplate(name.to_string()));
    }
    Ok(safe)
}

fn reject_secret_shapes(body: &str) -> WorkflowResult<()> {
    let lower = body.to_ascii_lowercase();
    let suspicious = [
        "authorization:",
        "bearer ",
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
        "password:",
        "secret:",
        "sk-",
    ];
    if let Some(hit) = suspicious.iter().find(|needle| lower.contains(**needle)) {
        return Err(WorkflowError::UnsafeTemplate(format!(
            "template contains credential-like text: {hit}"
        )));
    }
    Ok(())
}
