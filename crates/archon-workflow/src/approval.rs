use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::bundle::{COMPILED_SPEC_FILE, HARNESS_FILE, WorkflowBundle};
use crate::error::{WorkflowError, WorkflowResult};
use crate::run::WorkflowRun;
use crate::spec::StageKind;
use crate::store::WorkflowStore;

pub const APPROVALS_FILE: &str = "approvals.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowApprovalDecision {
    RunOnce,
    AlwaysForProject,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowApprovalRecord {
    pub workflow_hash: String,
    pub project_root: String,
    pub workflow_name: String,
    pub decision: WorkflowApprovalDecision,
    pub decided_at: String,
    pub decided_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub phase_count: usize,
    pub max_agents: u32,
    pub max_parallelism: u32,
    pub write_capable_stages: Vec<String>,
    pub external_requirements: Vec<String>,
    pub raw_script_path: String,
    pub compiled_spec_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowApprovalInspection {
    pub workflow_hash: String,
    pub project_root: String,
    pub workflow_name: String,
    pub phase_count: usize,
    pub max_agents: u32,
    pub max_parallelism: u32,
    pub write_capable_stages: Vec<String>,
    pub external_requirements: Vec<String>,
    pub cost_warning: String,
    pub raw_script_path: String,
    pub compiled_spec_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<WorkflowApprovalRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WorkflowApprovalFile {
    #[serde(default)]
    records: Vec<WorkflowApprovalRecord>,
}

#[derive(Debug, Clone)]
pub struct WorkflowApprovalStore {
    path: PathBuf,
}

impl WorkflowApprovalStore {
    pub fn project(project_root: impl AsRef<Path>) -> Self {
        Self {
            path: project_root
                .as_ref()
                .join(".archon")
                .join("workflows")
                .join(APPROVALS_FILE),
        }
    }

    pub fn for_workflow_store(store: &WorkflowStore) -> Self {
        Self::project(project_root_from_workflow_root(store.root()))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn inspect_run(
        &self,
        project_root: impl AsRef<Path>,
        store: &WorkflowStore,
        run: &WorkflowRun,
    ) -> WorkflowResult<WorkflowApprovalInspection> {
        let manifest = WorkflowBundle::verify(store, &run.id)?;
        let project_root = stable_project_root(project_root.as_ref());
        let raw_script_path = store
            .run_dir(&run.id)
            .join(HARNESS_FILE)
            .display()
            .to_string();
        let compiled_spec_path = store
            .run_dir(&run.id)
            .join(COMPILED_SPEC_FILE)
            .display()
            .to_string();
        let decision = self.latest_decision(&project_root, &manifest.workflow_hash)?;
        Ok(WorkflowApprovalInspection {
            workflow_hash: manifest.workflow_hash,
            project_root,
            workflow_name: manifest.name,
            phase_count: manifest.phase_count,
            max_agents: manifest.max_agents,
            max_parallelism: manifest.max_parallelism,
            write_capable_stages: manifest.write_capable_stages,
            external_requirements: external_requirements(run),
            cost_warning: cost_warning(run),
            raw_script_path,
            compiled_spec_path,
            decision,
        })
    }

    pub fn approve_run_once(
        &self,
        project_root: impl AsRef<Path>,
        store: &WorkflowStore,
        run: &WorkflowRun,
        decided_by: impl Into<String>,
    ) -> WorkflowResult<WorkflowApprovalRecord> {
        self.record_run_decision(
            project_root,
            store,
            run,
            WorkflowApprovalDecision::RunOnce,
            decided_by,
        )
    }

    pub fn approve_always_for_project(
        &self,
        project_root: impl AsRef<Path>,
        store: &WorkflowStore,
        run: &WorkflowRun,
        decided_by: impl Into<String>,
    ) -> WorkflowResult<WorkflowApprovalRecord> {
        self.record_run_decision(
            project_root,
            store,
            run,
            WorkflowApprovalDecision::AlwaysForProject,
            decided_by,
        )
    }

    pub fn deny_run(
        &self,
        project_root: impl AsRef<Path>,
        store: &WorkflowStore,
        run: &WorkflowRun,
        decided_by: impl Into<String>,
    ) -> WorkflowResult<WorkflowApprovalRecord> {
        self.record_run_decision(
            project_root,
            store,
            run,
            WorkflowApprovalDecision::Denied,
            decided_by,
        )
    }

    pub fn latest_decision(
        &self,
        project_root: &str,
        workflow_hash: &str,
    ) -> WorkflowResult<Option<WorkflowApprovalRecord>> {
        let file = self.load()?;
        Ok(file.records.into_iter().rev().find(|record| {
            record.project_root == project_root && record.workflow_hash == workflow_hash
        }))
    }

    fn record_run_decision(
        &self,
        project_root: impl AsRef<Path>,
        store: &WorkflowStore,
        run: &WorkflowRun,
        decision: WorkflowApprovalDecision,
        decided_by: impl Into<String>,
    ) -> WorkflowResult<WorkflowApprovalRecord> {
        let inspection = self.inspect_run(project_root, store, run)?;
        let record = WorkflowApprovalRecord {
            workflow_hash: inspection.workflow_hash,
            project_root: inspection.project_root,
            workflow_name: inspection.workflow_name,
            decision,
            decided_at: Utc::now().to_rfc3339(),
            decided_by: decided_by.into(),
            run_id: Some(run.id.clone()),
            phase_count: inspection.phase_count,
            max_agents: inspection.max_agents,
            max_parallelism: inspection.max_parallelism,
            write_capable_stages: inspection.write_capable_stages,
            external_requirements: inspection.external_requirements,
            raw_script_path: inspection.raw_script_path,
            compiled_spec_path: inspection.compiled_spec_path,
        };
        let mut file = self.load()?;
        file.records.retain(|existing| {
            !(existing.project_root == record.project_root
                && existing.workflow_hash == record.workflow_hash
                && existing.run_id == record.run_id
                && existing.decision == record.decision)
        });
        file.records.push(record.clone());
        self.save(&file)?;
        Ok(record)
    }

    fn load(&self) -> WorkflowResult<WorkflowApprovalFile> {
        if !self.path.exists() {
            return Ok(WorkflowApprovalFile::default());
        }
        let raw = fs::read(&self.path).map_err(|err| WorkflowError::io(&self.path, err))?;
        serde_json::from_slice(&raw).map_err(Into::into)
    }

    fn save(&self, file: &WorkflowApprovalFile) -> WorkflowResult<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| WorkflowError::io(parent, err))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(file)?;
        fs::write(&tmp, bytes).map_err(|err| WorkflowError::io(&tmp, err))?;
        fs::rename(&tmp, &self.path).map_err(|err| WorkflowError::io(&self.path, err))?;
        Ok(())
    }
}

pub fn project_root_from_workflow_root(workflow_root: &Path) -> PathBuf {
    let Some(archon_dir) = workflow_root.parent() else {
        return workflow_root.to_path_buf();
    };
    if archon_dir.file_name().and_then(|name| name.to_str()) == Some(".archon")
        && workflow_root.file_name().and_then(|name| name.to_str()) == Some("workflows")
    {
        return archon_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| workflow_root.to_path_buf());
    }
    workflow_root.to_path_buf()
}

fn stable_project_root(project_root: &Path) -> String {
    project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf())
        .display()
        .to_string()
}

fn external_requirements(run: &WorkflowRun) -> Vec<String> {
    let mut requirements = run
        .spec
        .stages
        .iter()
        .filter_map(|stage| {
            if stage.kind == StageKind::Tool {
                Some(format!(
                    "tool:{}",
                    stage.tool.as_deref().unwrap_or(stage.id.as_str())
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    requirements.sort();
    requirements.dedup();
    requirements
}

fn cost_warning(run: &WorkflowRun) -> String {
    format!(
        "May launch up to {} agents with max parallelism {}; live provider token and rate limits apply.",
        run.spec.max_agents, run.spec.max_parallelism
    )
}
