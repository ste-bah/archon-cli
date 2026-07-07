use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::bundle::{
    COMPILED_SPEC_FILE, HARNESS_FILE, MANIFEST_FILE, WorkflowBundle, WorkflowBundleManifest,
    WorkflowBundleOrigin, sanitize_command_name, sanitized_harness, user_command_dir,
    workflow_command_dir, write_capable_stage_ids,
};
use crate::error::{WorkflowError, WorkflowResult};
use crate::run::WorkflowRun;
use crate::spec::WorkflowSpec;
use crate::store::WorkflowStore;

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

#[derive(Debug, Clone, PartialEq)]
pub struct SavedWorkflowCommand {
    pub name: String,
    pub spec: WorkflowSpec,
    pub harness_source: String,
    pub manifest: WorkflowBundleManifest,
    pub command_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct WorkflowCommandRegistry {
    project_root: PathBuf,
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

impl WorkflowCommandRegistry {
    pub fn project(project_root: impl AsRef<Path>) -> Self {
        Self {
            project_root: project_root.as_ref().to_path_buf(),
        }
    }

    pub fn save_run(
        &self,
        name: &str,
        store: &WorkflowStore,
        run: &WorkflowRun,
    ) -> WorkflowResult<SavedWorkflowCommand> {
        WorkflowBundle::verify(store, &run.id)?;
        let safe = sanitize_command_name(name)?;
        let command_dir = workflow_command_dir(&self.project_root, &safe)?;
        fs::create_dir_all(&command_dir).map_err(|e| WorkflowError::io(&command_dir, e))?;
        let harness_path = store.run_dir(&run.id).join(HARNESS_FILE);
        let harness =
            fs::read_to_string(&harness_path).map_err(|e| WorkflowError::io(&harness_path, e))?;
        let harness = sanitized_harness(&harness)?;
        validate_saved_harness(&harness)?;
        let spec = sanitize_spec(&run.spec)?;
        let compiled = spec.to_yaml()?;
        let manifest = command_manifest(&safe, &spec, harness.as_bytes(), compiled.as_bytes());
        write_atomic(&command_dir.join(HARNESS_FILE), harness.as_bytes())?;
        write_atomic(&command_dir.join(COMPILED_SPEC_FILE), compiled.as_bytes())?;
        write_atomic(
            &command_dir.join(MANIFEST_FILE),
            toml::to_string_pretty(&manifest)?.as_bytes(),
        )?;
        Ok(SavedWorkflowCommand {
            name: safe,
            spec,
            harness_source: harness,
            manifest,
            command_dir,
        })
    }

    pub fn load(&self, name: &str) -> WorkflowResult<Option<SavedWorkflowCommand>> {
        let safe = sanitize_command_name(name)?;
        let Some(command_dir) = self.resolve(&safe)? else {
            return Ok(None);
        };
        let harness_path = command_dir.join(HARNESS_FILE);
        let compiled_path = command_dir.join(COMPILED_SPEC_FILE);
        let manifest_path = command_dir.join(MANIFEST_FILE);
        let harness_source =
            fs::read_to_string(&harness_path).map_err(|e| WorkflowError::io(&harness_path, e))?;
        validate_saved_harness(&harness_source)?;
        let compiled =
            fs::read_to_string(&compiled_path).map_err(|e| WorkflowError::io(&compiled_path, e))?;
        reject_secret_shapes(&compiled)?;
        let spec = WorkflowSpec::from_yaml(&compiled)?;
        let manifest_raw =
            fs::read_to_string(&manifest_path).map_err(|e| WorkflowError::io(&manifest_path, e))?;
        let manifest: WorkflowBundleManifest = toml::from_str(&manifest_raw)?;
        let expected =
            command_manifest(&safe, &spec, harness_source.as_bytes(), compiled.as_bytes());
        if manifest.workflow_hash != expected.workflow_hash
            || manifest.compiled_hash != expected.compiled_hash
        {
            return Err(WorkflowError::ArtifactInvalid(format!(
                "saved workflow command `{safe}` failed hash verification"
            )));
        }
        Ok(Some(SavedWorkflowCommand {
            name: safe,
            spec,
            harness_source,
            manifest,
            command_dir,
        }))
    }

    fn resolve(&self, safe: &str) -> WorkflowResult<Option<PathBuf>> {
        for ancestor in self.project_root.ancestors() {
            let candidate = workflow_command_dir(ancestor, safe)?;
            if candidate.join(MANIFEST_FILE).exists() {
                return Ok(Some(candidate));
            }
        }
        let user = user_command_dir(safe)?;
        if user.join(MANIFEST_FILE).exists() {
            return Ok(Some(user));
        }
        Ok(None)
    }
}

fn validate_saved_harness(source: &str) -> WorkflowResult<()> {
    // Source-text scanning is gone: QuickJS is the single grammar and the
    // sandbox is the engine. The dry-run at the run boundary is the real
    // validation; saving only requires a non-empty script.
    if source.trim().is_empty() {
        return Err(WorkflowError::UnsafeTemplate(
            "saved workflow harness is empty".to_string(),
        ));
    }
    Ok(())
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

fn command_manifest(
    name: &str,
    spec: &WorkflowSpec,
    harness: &[u8],
    compiled: &[u8],
) -> WorkflowBundleManifest {
    WorkflowBundleManifest {
        id: format!("command:{name}"),
        name: spec.name.clone(),
        schema: spec.schema.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        origin: WorkflowBundleOrigin::SavedCommand,
        workflow_hash: blake3::hash(harness).to_hex().to_string(),
        compiled_hash: blake3::hash(compiled).to_hex().to_string(),
        phase_count: spec.stages.len(),
        max_agents: spec.max_agents,
        max_parallelism: spec.max_parallelism,
        write_capable_stages: write_capable_stage_ids(spec),
        command_name: Some(name.to_string()),
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> WorkflowResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| WorkflowError::io(parent, e))?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| WorkflowError::io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| WorkflowError::io(path, e))
}
