use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::error::{WorkflowError, WorkflowResult};
use crate::events::sanitize_value;
use crate::run::WorkflowRun;
use crate::spec::{StageKind, StageSpec, WorkflowSpec};
use crate::store::WorkflowStore;

pub const MANIFEST_FILE: &str = "manifest.toml";
pub const HARNESS_FILE: &str = "workflow.js";
pub const COMPILED_SPEC_FILE: &str = "workflow.compiled.yaml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowBundleOrigin {
    GeneratedHarness,
    ImportedSpecWrapper,
    SavedCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowBundleManifest {
    pub id: String,
    pub name: String,
    pub schema: String,
    pub created_at: String,
    pub origin: WorkflowBundleOrigin,
    pub workflow_hash: String,
    pub compiled_hash: String,
    pub phase_count: usize,
    pub max_agents: u32,
    pub max_parallelism: u32,
    pub write_capable_stages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowBundle {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub manifest: WorkflowBundleManifest,
}

impl WorkflowBundle {
    pub fn create_for_run(
        store: &WorkflowStore,
        run: &WorkflowRun,
        harness_source: &str,
        origin: WorkflowBundleOrigin,
    ) -> WorkflowResult<Self> {
        let compiled = run.spec.to_yaml()?;
        let manifest =
            manifest_for_run(run, harness_source.as_bytes(), compiled.as_bytes(), origin);
        store.write_run_file(&run.id, HARNESS_FILE, harness_source.as_bytes())?;
        store.write_run_file(&run.id, COMPILED_SPEC_FILE, compiled.as_bytes())?;
        store.write_run_file(
            &run.id,
            MANIFEST_FILE,
            toml::to_string_pretty(&manifest)?.as_bytes(),
        )?;
        Ok(Self {
            run_id: run.id.clone(),
            run_dir: store.run_dir(&run.id),
            manifest,
        })
    }

    pub fn synthesize_for_imported_spec(
        store: &WorkflowStore,
        run: &WorkflowRun,
    ) -> WorkflowResult<Self> {
        let harness = WorkflowHarness::imported_spec_wrapper(COMPILED_SPEC_FILE);
        Self::create_for_run(
            store,
            run,
            &harness.source,
            WorkflowBundleOrigin::ImportedSpecWrapper,
        )
    }

    pub fn verify(store: &WorkflowStore, run_id: &str) -> WorkflowResult<WorkflowBundleManifest> {
        let manifest = read_manifest(store, run_id)?;
        let harness = std::fs::read(store.run_dir(run_id).join(HARNESS_FILE))
            .map_err(|err| WorkflowError::io(store.run_dir(run_id).join(HARNESS_FILE), err))?;
        let compiled =
            std::fs::read(store.run_dir(run_id).join(COMPILED_SPEC_FILE)).map_err(|err| {
                WorkflowError::io(store.run_dir(run_id).join(COMPILED_SPEC_FILE), err)
            })?;
        let workflow_hash = content_hash(&harness);
        let compiled_hash = content_hash(&compiled);
        if workflow_hash != manifest.workflow_hash {
            return Err(WorkflowError::ArtifactInvalid(format!(
                "workflow harness hash mismatch for run {run_id}"
            )));
        }
        if compiled_hash != manifest.compiled_hash {
            return Err(WorkflowError::ArtifactInvalid(format!(
                "compiled workflow hash mismatch for run {run_id}"
            )));
        }
        Ok(manifest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowHarness {
    pub source: String,
}

impl WorkflowHarness {
    pub fn from_spec(spec: &WorkflowSpec) -> Self {
        Self {
            source: render_harness_from_spec(spec),
        }
    }

    pub fn imported_spec_wrapper(compiled_spec: &str) -> Self {
        Self {
            source: format!(
                "export default async function workflow(w) {{\n  return w.runCompiledSpec({});\n}}\n",
                js_string(compiled_spec)
            ),
        }
    }
}

pub fn read_manifest(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<WorkflowBundleManifest> {
    let path = store.run_dir(run_id).join(MANIFEST_FILE);
    let raw = std::fs::read_to_string(&path).map_err(|err| WorkflowError::io(&path, err))?;
    toml::from_str(&raw).map_err(Into::into)
}

pub fn workflow_command_dir(project_root: &Path, name: &str) -> WorkflowResult<PathBuf> {
    Ok(project_root
        .join(".archon")
        .join("workflows")
        .join("commands")
        .join(sanitize_command_name(name)?))
}

pub fn user_command_dir(name: &str) -> WorkflowResult<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| WorkflowError::UnsafeTemplate("HOME is not set".to_string()))?;
    Ok(home
        .join(".archon")
        .join("workflows")
        .join("commands")
        .join(sanitize_command_name(name)?))
}

pub fn sanitize_command_name(name: &str) -> WorkflowResult<String> {
    let safe: String = name
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if safe.is_empty() || safe.contains("..") {
        return Err(WorkflowError::UnsafeTemplate(name.to_string()));
    }
    Ok(safe)
}

pub fn sanitized_harness(source: &str) -> WorkflowResult<String> {
    reject_secret_shapes(source)?;
    let cleaned = sanitize_value(serde_json::json!({ "source": strip_stale_run_ids(source) }));
    Ok(cleaned
        .get("source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(source)
        .to_string())
}

fn strip_stale_run_ids(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let chars = source.char_indices().collect::<Vec<_>>();
    let mut idx = 0usize;
    while idx < chars.len() {
        let start = chars[idx].0;
        if source[start..].starts_with("wf-") {
            let end_idx = chars[idx..]
                .iter()
                .position(|(_, ch)| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')))
                .map(|offset| idx + offset)
                .unwrap_or(chars.len());
            let end = chars
                .get(end_idx)
                .map(|(pos, _)| *pos)
                .unwrap_or(source.len());
            let candidate = &source[start..end];
            if looks_like_workflow_run_id(candidate) {
                out.push_str("{run_id}");
                idx = end_idx;
                continue;
            }
        }
        out.push(chars[idx].1);
        idx += 1;
    }
    out
}

fn looks_like_workflow_run_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("wf-") else {
        return false;
    };
    let parts = rest.split('-').collect::<Vec<_>>();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(parts.iter())
            .all(|(len, part)| part.len() == *len && part.chars().all(|ch| ch.is_ascii_hexdigit()))
}

fn manifest_for_run(
    run: &WorkflowRun,
    harness: &[u8],
    compiled: &[u8],
    origin: WorkflowBundleOrigin,
) -> WorkflowBundleManifest {
    WorkflowBundleManifest {
        id: run.id.clone(),
        name: run.spec.name.clone(),
        schema: run.spec.schema.clone(),
        created_at: Utc::now().to_rfc3339(),
        origin,
        workflow_hash: content_hash(harness),
        compiled_hash: content_hash(compiled),
        phase_count: run.spec.stages.len(),
        max_agents: run.spec.max_agents,
        max_parallelism: run.spec.max_parallelism,
        write_capable_stages: write_capable_stage_ids(&run.spec),
        command_name: None,
    }
}

pub(crate) fn write_capable_stage_ids(spec: &WorkflowSpec) -> Vec<String> {
    spec.stages
        .iter()
        .filter(|stage| stage_is_write_capable(stage))
        .map(|stage| stage.id.clone())
        .collect()
}

fn stage_is_write_capable(stage: &StageSpec) -> bool {
    stage.kind == StageKind::Implementation
        || stage.item_kind == Some(StageKind::Implementation)
        || stage
            .input
            .get("write_mode")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|mode| matches!(mode, "serial" | "coordinated" | "worktree"))
}

fn render_harness_from_spec(spec: &WorkflowSpec) -> String {
    let mut out = String::new();
    out.push_str("export default async function workflow(w) {\n");
    out.push_str(
        "  await w.checkpoint(\"bundle-start\", { compiled: \"workflow.compiled.yaml\" });\n",
    );
    for stage in &spec.stages {
        let task = stage.task.as_deref().unwrap_or(&spec.task);
        let tier = stage
            .provider_tier
            .map(|tier| format!("{tier:?}").to_ascii_lowercase())
            .unwrap_or_else(|| "auto".to_string());
        let deps = serde_json::to_string(&stage.depends_on).unwrap_or_else(|_| "[]".to_string());
        let call = match stage.kind {
            StageKind::Agent => "agent",
            StageKind::Fanout => "fanout",
            StageKind::Reduce => "reduce",
            StageKind::Tool => "tool",
            StageKind::Implementation => "implementation",
            StageKind::QualityGate => "qualityGate",
            StageKind::HumanGate => "humanGate",
            StageKind::Checkpoint => "checkpoint",
        };
        out.push_str(&format!(
            "  await w.{call}({}, {{ tier: {}, task: {}, depends_on: {}, write: {} }});\n",
            js_string(&stage.id),
            js_string(&tier),
            js_string(task),
            deps,
            stage.kind == StageKind::Implementation
                || stage.item_kind == Some(StageKind::Implementation)
        ));
    }
    out.push_str("}\n");
    out
}

fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn js_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
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
            "workflow command contains credential-like text: {hit}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitized_harness_strips_stale_run_ids_without_collapsing_script_text() {
        let source = "export default async function workflow(w) {\n  const previous = \"wf-12345678-1234-1234-1234-123456789abc\";\n  return await w.agent(\"discover\", { task: previous });\n}\n";

        let clean = sanitized_harness(source).expect("sanitized harness");

        assert!(!clean.contains("wf-12345678-1234-1234-1234-123456789abc"));
        assert!(clean.contains("{run_id}"));
        assert!(clean.contains("export default async function workflow(w) {\n"));
        assert!(clean.contains("  return await w.agent"));
    }

    #[test]
    fn sanitized_harness_rejects_credential_shapes() {
        let error = sanitized_harness("export default async function workflow(w) { return \"sk-ant-secretsecretsecretsecretsecretsecretsecretsecret\"; }")
            .expect_err("secret should be rejected");

        assert!(error.to_string().contains("credential-like text"));
    }
}
