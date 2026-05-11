//! Backend-specific checkpoint metadata.

use std::path::{Path, PathBuf};

use anyhow::Result;
use safetensors::tensor::{Dtype, TensorView, serialize_to_file};
use serde::{Deserialize, Serialize};

use crate::backend::BackendKind;
use crate::model::CpuLatentTransitionModel;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointFormat {
    CandleSafetensors,
    MlxArray,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub model_id: String,
    pub backend: BackendKind,
    pub format: CheckpointFormat,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandleCheckpointTensors {
    pub state_weights: Vec<f32>,
    pub action_weights: Vec<f32>,
    pub transition_bias: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MlxArrayCheckpoint {
    pub model_id: String,
    pub arrays: CandleCheckpointTensors,
    pub memory_order: String,
    pub dtype: String,
}

impl CheckpointRecord {
    pub fn candidate(root: &Path, model_id: impl Into<String>, backend: BackendKind) -> Self {
        let model_id = model_id.into();
        let format = checkpoint_format(backend);
        let extension = match format {
            CheckpointFormat::CandleSafetensors => "safetensors",
            CheckpointFormat::MlxArray => "mlx",
        };
        Self {
            path: root
                .join("candidates")
                .join(format!("{model_id}.{extension}")),
            model_id,
            backend,
            format,
        }
    }
}

pub fn write_candle_safetensors_checkpoint(
    root: &Path,
    model: &CpuLatentTransitionModel,
) -> Result<CheckpointRecord> {
    let record = CheckpointRecord::candidate(root, &model.metadata.model_id, BackendKind::Cpu);
    if let Some(parent) = record.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tensors = [
        (
            "state_weights".to_string(),
            f32_bytes(&model.state_weights),
            model.state_weights.len(),
        ),
        (
            "action_weights".to_string(),
            f32_bytes(&model.action_weights),
            model.action_weights.len(),
        ),
        (
            "transition_bias".to_string(),
            f32_bytes(&model.transition_bias),
            model.transition_bias.len(),
        ),
    ];
    let views = tensors
        .iter()
        .map(|(name, bytes, len)| {
            Ok((
                name.clone(),
                TensorView::new(Dtype::F32, vec![*len], bytes.as_slice())?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    serialize_to_file(views, None, &record.path)?;
    Ok(record)
}

pub fn read_candle_safetensors_checkpoint(path: &Path) -> Result<CandleCheckpointTensors> {
    let bytes = std::fs::read(path)?;
    let tensors = safetensors::SafeTensors::deserialize(&bytes)?;
    Ok(CandleCheckpointTensors {
        state_weights: tensor_f32(&tensors, "state_weights")?,
        action_weights: tensor_f32(&tensors, "action_weights")?,
        transition_bias: tensor_f32(&tensors, "transition_bias")?,
    })
}

pub fn write_mlx_array_checkpoint(
    root: &Path,
    model: &CpuLatentTransitionModel,
) -> Result<CheckpointRecord> {
    let record = CheckpointRecord::candidate(root, &model.metadata.model_id, BackendKind::Metal);
    if let Some(parent) = record.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let checkpoint = MlxArrayCheckpoint {
        model_id: model.metadata.model_id.clone(),
        arrays: CandleCheckpointTensors {
            state_weights: model.state_weights.clone(),
            action_weights: model.action_weights.clone(),
            transition_bias: model.transition_bias.clone(),
        },
        memory_order: "row_major".into(),
        dtype: "f32".into(),
    };
    std::fs::write(&record.path, serde_json::to_vec_pretty(&checkpoint)?)?;
    Ok(record)
}

pub fn read_mlx_array_checkpoint(path: &Path) -> Result<MlxArrayCheckpoint> {
    let content = std::fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(Into::into)
}

pub fn checkpoint_format(backend: BackendKind) -> CheckpointFormat {
    match backend {
        BackendKind::Metal => CheckpointFormat::MlxArray,
        BackendKind::Auto | BackendKind::Cpu | BackendKind::Cuda => {
            CheckpointFormat::CandleSafetensors
        }
    }
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn tensor_f32(tensors: &safetensors::SafeTensors<'_>, name: &str) -> Result<Vec<f32>> {
    let tensor = tensors.tensor(name)?;
    Ok(tensor
        .data()
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_and_cuda_use_candle_checkpoint_format() {
        assert_eq!(
            checkpoint_format(BackendKind::Cpu),
            CheckpointFormat::CandleSafetensors
        );
        assert_eq!(
            checkpoint_format(BackendKind::Cuda),
            CheckpointFormat::CandleSafetensors
        );
    }

    #[test]
    fn metal_uses_mlx_checkpoint_format() {
        let root = PathBuf::from("/tmp/world-model");
        let record = CheckpointRecord::candidate(&root, "model-1", BackendKind::Metal);

        assert_eq!(record.format, CheckpointFormat::MlxArray);
        assert!(record.path.ends_with("model-1.mlx"));
    }

    #[test]
    fn candle_safetensors_checkpoint_roundtrips_weights() {
        let temp = tempfile::tempdir().unwrap();
        let examples = [crate::model::LatentTransitionExample {
            state: vec![0.0, 0.0],
            action: vec![0.0, 0.0],
            next_state: vec![1.0, 1.0],
            labels: Default::default(),
        }];
        let model = CpuLatentTransitionModel::fit(2, &examples).unwrap();

        let record = write_candle_safetensors_checkpoint(temp.path(), &model).unwrap();
        let loaded = read_candle_safetensors_checkpoint(&record.path).unwrap();

        assert_eq!(record.format, CheckpointFormat::CandleSafetensors);
        assert_eq!(loaded.transition_bias, model.transition_bias);
    }

    #[test]
    fn mlx_array_checkpoint_roundtrips_weights() {
        let temp = tempfile::tempdir().unwrap();
        let examples = [crate::model::LatentTransitionExample {
            state: vec![0.0, 0.0],
            action: vec![0.0, 0.0],
            next_state: vec![1.0, 1.0],
            labels: Default::default(),
        }];
        let model = CpuLatentTransitionModel::fit(2, &examples).unwrap();

        let record = write_mlx_array_checkpoint(temp.path(), &model).unwrap();
        let loaded = read_mlx_array_checkpoint(&record.path).unwrap();

        assert_eq!(record.format, CheckpointFormat::MlxArray);
        assert_eq!(loaded.arrays.transition_bias, model.transition_bias);
        assert_eq!(loaded.memory_order, "row_major");
    }
}
