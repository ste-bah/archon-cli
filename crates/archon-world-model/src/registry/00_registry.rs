use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;

use crate::eval::{
    BrierImprovementReport, NextStateCosineGateReport, PromotionGateReport, SurpriseKsReport,
};
use crate::jepa::{
    JepaCheckpointRecord, JepaEvalRecord, JepaRepresentationComparisonReport, JepaTraceModel,
    JepaTrainingOutcome,
};
use crate::model::CpuLatentTransitionModel;
use crate::train::TrainingOutcome;

pub const LATENT_TRANSITION_MODEL_KIND: &str = "latent_transition";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRegistryPaths {
    pub root: PathBuf,
    pub candidates_dir: PathBuf,
    pub active_dir: PathBuf,
    pub ledgers_dir: PathBuf,
}

impl ModelRegistryPaths {
    pub fn under(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        Self {
            candidates_dir: root.join("candidates"),
            active_dir: root.join("active"),
            ledgers_dir: root.join("ledgers"),
            root,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveModelPointer {
    pub model_id: String,
    #[serde(
        default = "default_model_kind",
        skip_serializing_if = "is_default_model_kind"
    )]
    pub model_kind: String,
    pub previous_model_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelActivationRecord {
    pub model_id: String,
    #[serde(
        default = "default_model_kind",
        skip_serializing_if = "is_default_model_kind"
    )]
    pub model_kind: String,
    pub previous_model_id: Option<String>,
    pub action: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuCandidateRecord {
    pub model: CpuLatentTransitionModel,
    pub outcome: TrainingOutcome,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JepaCandidateRecord {
    pub model: JepaTraceModel,
    pub outcome: JepaTrainingOutcome,
    pub checkpoint: JepaCheckpointRecord,
    #[serde(default)]
    pub training_run: PathBuf,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateEvalRecord {
    pub candidate_id: String,
    pub report: PromotionGateReport,
    pub next_state: Option<NextStateCosineGateReport>,
    pub surprise: Option<SurpriseKsReport>,
    pub brier: Option<BrierImprovementReport>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ModelRegistry {
    paths: ModelRegistryPaths,
}

impl ModelRegistry {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        // Canonicalize root for the same reason as WorldModelStore::open:
        // macOS tempdir paths involve a /private symlink that produces
        // inconsistent path identity across reopens.
        std::fs::create_dir_all(root.as_ref())?;
        let root = std::fs::canonicalize(root.as_ref())?;
        let paths = ModelRegistryPaths::under(&root);
        std::fs::create_dir_all(&paths.candidates_dir)?;
        std::fs::create_dir_all(&paths.active_dir)?;
        std::fs::create_dir_all(&paths.ledgers_dir)?;
        Ok(Self { paths })
    }

    pub fn active_model_id(&self) -> Result<Option<String>> {
        let path = self.active_pointer_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)?;
        let pointer: ActiveModelPointer = serde_json::from_str(&content)?;
        Ok(Some(pointer.model_id))
    }

    pub fn promote(&self, model_id: impl Into<String>) -> Result<PathBuf> {
        self.promote_model_kind(model_id, LATENT_TRANSITION_MODEL_KIND)
    }

    pub fn active_model_kind(&self) -> Result<Option<String>> {
        let path = self.active_pointer_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)?;
        let pointer: ActiveModelPointer = serde_json::from_str(&content)?;
        Ok(Some(pointer.model_kind))
    }

    pub fn promote_model_kind(
        &self,
        model_id: impl Into<String>,
        model_kind: impl Into<String>,
    ) -> Result<PathBuf> {
        let previous_model_id = self.active_model_id()?;
        let pointer = ActiveModelPointer {
            model_id: model_id.into(),
            model_kind: model_kind.into(),
            previous_model_id,
            updated_at: Utc::now(),
        };
        let path = self.active_pointer_path();
        std::fs::write(&path, serde_json::to_vec_pretty(&pointer)?)?;
        self.append_activation(&pointer, "promote")?;
        Ok(path)
    }

    pub fn write_cpu_candidate(
        &self,
        model: &CpuLatentTransitionModel,
        outcome: &TrainingOutcome,
    ) -> Result<PathBuf> {
        self.write_candidate(model, outcome)
    }

    pub fn write_candidate(
        &self,
        model: &CpuLatentTransitionModel,
        outcome: &TrainingOutcome,
    ) -> Result<PathBuf> {
        let record = CpuCandidateRecord {
            model: model.clone(),
            outcome: outcome.clone(),
            created_at: Utc::now(),
        };
        let path = self.candidate_record_path(&record.model.metadata.model_id);
        std::fs::write(&path, serde_json::to_vec_pretty(&record)?)?;
        match model.metadata.backend {
            crate::backend::BackendKind::Metal => {
                crate::checkpoint::write_mlx_array_checkpoint(&self.paths.root, model)?;
            }
            crate::backend::BackendKind::Auto
            | crate::backend::BackendKind::Cpu
            | crate::backend::BackendKind::Cuda => {
                crate::checkpoint::write_candle_safetensors_checkpoint(&self.paths.root, model)?;
            }
        }
        Ok(path)
    }

    pub fn load_cpu_candidate(&self, model_id: &str) -> Result<CpuCandidateRecord> {
        let content = std::fs::read_to_string(self.candidate_record_path(model_id))?;
        serde_json::from_str(&content).map_err(Into::into)
    }

    pub fn write_jepa_candidate(
        &self,
        model: &JepaTraceModel,
        outcome: &JepaTrainingOutcome,
    ) -> Result<PathBuf> {
        self.ensure_jepa_dirs()?;
        crate::jepa::validate_jepa_backend_execution(&model.metadata)?;
        crate::jepa::validate_jepa_backend_execution(&outcome.metadata)?;
        let checkpoint = crate::jepa::write_jepa_checkpoint(&self.paths.root, model)?;
        let training_run = crate::jepa::append_jepa_training_run(&self.paths.root, outcome)?;
        let record = JepaCandidateRecord {
            model: model.clone(),
            outcome: outcome.clone(),
            checkpoint,
            training_run,
            created_at: Utc::now(),
        };
        let path = self.jepa_candidate_record_path(&record.model.metadata.model_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&record)?)?;
        Ok(path)
    }

    pub fn load_jepa_candidate(&self, model_id: &str) -> Result<JepaCandidateRecord> {
        let content = std::fs::read_to_string(self.jepa_candidate_record_path(model_id))?;
        serde_json::from_str(&content).map_err(Into::into)
    }

    pub fn write_jepa_eval_report(&self, record: &JepaEvalRecord) -> Result<PathBuf> {
        self.ensure_jepa_dirs()?;
        let path = self.jepa_eval_record_path(&record.candidate_id);
        std::fs::write(&path, serde_json::to_vec_pretty(record)?)?;
        Ok(path)
    }

    pub fn load_jepa_eval_report(&self, model_id: &str) -> Result<Option<JepaEvalRecord>> {
        let path = self.jepa_eval_record_path(model_id);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).map(Some).map_err(Into::into)
    }

    pub fn write_jepa_representation_comparison(
        &self,
        record: &JepaRepresentationComparisonReport,
    ) -> Result<PathBuf> {
        self.ensure_jepa_dirs()?;
        let path = self.jepa_representation_comparison_path(&record.candidate_id);
        std::fs::write(&path, serde_json::to_vec_pretty(record)?)?;
        Ok(path)
    }

    pub fn write_eval_report(&self, record: &CandidateEvalRecord) -> Result<PathBuf> {
        let path = self.eval_record_path(&record.candidate_id);
        std::fs::write(&path, serde_json::to_vec_pretty(record)?)?;
        Ok(path)
    }

    pub fn load_eval_report(&self, model_id: &str) -> Result<Option<CandidateEvalRecord>> {
        let path = self.eval_record_path(model_id);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).map(Some).map_err(Into::into)
    }

    pub fn rollback(&self, model_id: impl Into<String>) -> Result<PathBuf> {
        self.rollback_model_kind(model_id, LATENT_TRANSITION_MODEL_KIND)
    }

    pub fn rollback_model_kind(
        &self,
        model_id: impl Into<String>,
        model_kind: impl Into<String>,
    ) -> Result<PathBuf> {
        let previous_model_id = self.active_model_id()?;
        let pointer = ActiveModelPointer {
            model_id: model_id.into(),
            model_kind: model_kind.into(),
            previous_model_id,
            updated_at: Utc::now(),
        };
        let path = self.active_pointer_path();
        std::fs::write(&path, serde_json::to_vec_pretty(&pointer)?)?;
        self.append_activation(&pointer, "rollback")?;
        Ok(path)
    }

    pub fn candidate_count(&self) -> Result<usize> {
        let mut count = 0;
        for entry in std::fs::read_dir(&self.paths.candidates_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".eval.json"))
            {
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn latest_eval_report(&self) -> Result<Option<CandidateEvalRecord>> {
        let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
        for entry in std::fs::read_dir(&self.paths.candidates_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".eval.json"))
            {
                continue;
            }
            let modified = entry
                .metadata()?
                .modified()
                .unwrap_or(std::time::UNIX_EPOCH);
            if newest
                .as_ref()
                .is_none_or(|(current, _)| modified > *current)
            {
                newest = Some((modified, path));
            }
        }
        let Some((_, path)) = newest else {
            return Ok(None);
        };
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).map(Some).map_err(Into::into)
    }

    pub fn active_pointer_path(&self) -> PathBuf {
        self.paths.active_dir.join("model.json")
    }

    pub fn candidate_record_path(&self, model_id: &str) -> PathBuf {
        self.paths.candidates_dir.join(format!("{model_id}.json"))
    }

    pub fn eval_record_path(&self, model_id: &str) -> PathBuf {
        self.paths
            .candidates_dir
            .join(format!("{model_id}.eval.json"))
    }

    pub fn jepa_candidate_record_path(&self, model_id: &str) -> PathBuf {
        self.paths
            .root
            .join("jepa")
            .join("candidates")
            .join(format!("{model_id}.json"))
    }

    pub fn jepa_eval_record_path(&self, model_id: &str) -> PathBuf {
        self.paths
            .root
            .join("jepa")
            .join("evals")
            .join(format!("{model_id}.json"))
    }

    pub fn jepa_representation_comparison_path(&self, model_id: &str) -> PathBuf {
        self.paths
            .root
            .join("jepa")
            .join("representation-comparisons")
            .join(format!("{model_id}.json"))
    }

    fn append_activation(&self, pointer: &ActiveModelPointer, action: &str) -> Result<()> {
        let record = ModelActivationRecord {
            model_id: pointer.model_id.clone(),
            model_kind: pointer.model_kind.clone(),
            previous_model_id: pointer.previous_model_id.clone(),
            action: action.to_string(),
            created_at: pointer.updated_at,
        };
        let path = self.paths.ledgers_dir.join("model-activations.jsonl");
        let mut line = serde_json::to_vec(&record)?;
        line.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?
            .write_all(&line)?;
        Ok(())
    }

    fn ensure_jepa_dirs(&self) -> Result<()> {
        std::fs::create_dir_all(self.paths.root.join("jepa").join("candidates"))?;
        std::fs::create_dir_all(self.paths.root.join("jepa").join("evals"))?;
        std::fs::create_dir_all(self.paths.root.join("jepa").join("training-runs"))?;
        std::fs::create_dir_all(
            self.paths
                .root
                .join("jepa")
                .join("representation-comparisons"),
        )?;
        Ok(())
    }
}

fn default_model_kind() -> String {
    LATENT_TRANSITION_MODEL_KIND.into()
}

fn is_default_model_kind(model_kind: &String) -> bool {
    model_kind == LATENT_TRANSITION_MODEL_KIND
}

