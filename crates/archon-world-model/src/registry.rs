use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;

use crate::eval::{
    BrierImprovementReport, NextStateCosineGateReport, PromotionGateReport, SurpriseKsReport,
};
use crate::model::CpuLatentTransitionModel;
use crate::train::TrainingOutcome;

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
    pub previous_model_id: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelActivationRecord {
    pub model_id: String,
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
        let paths = ModelRegistryPaths::under(root);
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
        let previous_model_id = self.active_model_id()?;
        let pointer = ActiveModelPointer {
            model_id: model_id.into(),
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
        let previous_model_id = self.active_model_id()?;
        let pointer = ActiveModelPointer {
            model_id: model_id.into(),
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

    fn append_activation(&self, pointer: &ActiveModelPointer, action: &str) -> Result<()> {
        let record = ModelActivationRecord {
            model_id: pointer.model_id.clone(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_promotes_and_reads_active_model() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();

        let path = registry.promote("candidate-1").unwrap();

        assert!(path.exists());
        assert_eq!(
            registry.active_model_id().unwrap().as_deref(),
            Some("candidate-1")
        );
        assert!(
            temp.path()
                .join("ledgers")
                .join("model-activations.jsonl")
                .exists()
        );
    }

    #[test]
    fn registry_rollback_updates_active_pointer() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();

        registry.promote("candidate-1").unwrap();
        registry.rollback("candidate-0").unwrap();

        assert_eq!(
            registry.active_model_id().unwrap().as_deref(),
            Some("candidate-0")
        );
        let content =
            std::fs::read_to_string(temp.path().join("ledgers/model-activations.jsonl")).unwrap();
        assert!(content.contains("\"action\":\"rollback\""));
    }

    #[test]
    fn registry_writes_and_loads_cpu_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        let examples = [crate::model::LatentTransitionExample {
            state: vec![0.0, 0.0],
            action: vec![0.0, 0.0],
            next_state: vec![1.0, 1.0],
            labels: Default::default(),
        }];
        let (model, outcome) = crate::train::train_cpu_candidate(2, &examples).unwrap();

        let path = registry.write_cpu_candidate(&model, &outcome).unwrap();
        let loaded = registry
            .load_cpu_candidate(&model.metadata.model_id)
            .unwrap();

        assert!(path.exists());
        assert!(
            temp.path()
                .join("candidates")
                .join(format!("{}.safetensors", model.metadata.model_id))
                .exists()
        );
        assert_eq!(loaded.model.transition_bias, model.transition_bias);
        assert_eq!(loaded.outcome.status, outcome.status);
    }

    #[test]
    fn registry_writes_and_loads_eval_report() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        let record = CandidateEvalRecord {
            candidate_id: "candidate-1".into(),
            report: PromotionGateReport {
                cosine_error_improved: true,
                surprise_ks_passed: true,
                counterfactual_ndcg_passed: true,
                brier_improved: true,
                no_critical_regression: true,
            },
            next_state: None,
            surprise: None,
            brier: None,
            created_at: Utc::now(),
        };

        registry.write_eval_report(&record).unwrap();
        let loaded = registry.load_eval_report("candidate-1").unwrap().unwrap();

        assert!(loaded.report.all_primary_gates_passed());
    }

    #[test]
    fn registry_counts_candidate_records_only() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        std::fs::write(registry.candidate_record_path("c1"), "{}").unwrap();
        std::fs::write(registry.eval_record_path("c1"), "{}").unwrap();

        assert_eq!(registry.candidate_count().unwrap(), 1);
    }

    #[test]
    fn registry_loads_latest_eval_report() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        let record = CandidateEvalRecord {
            candidate_id: "candidate-1".into(),
            report: PromotionGateReport {
                cosine_error_improved: true,
                surprise_ks_passed: true,
                counterfactual_ndcg_passed: true,
                brier_improved: true,
                no_critical_regression: true,
            },
            next_state: None,
            surprise: None,
            brier: None,
            created_at: Utc::now(),
        };

        registry.write_eval_report(&record).unwrap();

        let loaded = registry.latest_eval_report().unwrap().unwrap();
        assert_eq!(loaded.candidate_id, "candidate-1");
    }
}
