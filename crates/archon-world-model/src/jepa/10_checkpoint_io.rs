pub fn append_jepa_training_run(root: &Path, outcome: &JepaTrainingOutcome) -> Result<PathBuf> {
    let dir = root.join("jepa").join("training-runs");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("training-runs.jsonl");
    let mut line = serde_json::to_vec(&serde_json::json!({
        "model_id": outcome.metadata.model_id.clone(),
        "model_kind": outcome.metadata.model_kind.clone(),
        "created_at": Utc::now(),
        "row_count": outcome.metadata.row_count,
        "example_count": outcome.metadata.example_count,
        "horizons": outcome.metadata.prediction_horizons.clone(),
        "backend_execution": outcome.metadata.backend_execution.clone(),
        "masking": outcome.masking.clone(),
        "initial_losses": outcome.initial_losses.clone(),
        "losses": outcome.losses.clone(),
        "progress": outcome.progress.clone(),
        "collapse": outcome.collapse.clone(),
        "horizon": outcome.horizon.clone()
    }))?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(&line)?;
    Ok(path)
}

pub fn write_jepa_safetensors_checkpoint(
    root: &Path,
    model: &JepaTraceModel,
) -> Result<JepaCheckpointRecord> {
    let record = JepaCheckpointRecord {
        model_id: model.metadata.model_id.clone(),
        format: "candle_safetensors".into(),
        path: root
            .join("jepa")
            .join("candidates")
            .join(format!("{}.safetensors", model.metadata.model_id)),
    };
    if let Some(parent) = record.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tensors = jepa_checkpoint_tensors(model);
    let named = vec![
        ("context_input_weights", tensors.context_input_weights),
        ("context_hidden_bias", tensors.context_hidden_bias),
        ("context_output_weights", tensors.context_output_weights),
        ("context_output_bias", tensors.context_output_bias),
        ("action_input_weights", tensors.action_input_weights),
        ("action_hidden_bias", tensors.action_hidden_bias),
        ("action_output_weights", tensors.action_output_weights),
        ("action_output_bias", tensors.action_output_bias),
        ("target_input_weights", tensors.target_input_weights),
        ("target_hidden_bias", tensors.target_hidden_bias),
        ("target_output_weights", tensors.target_output_weights),
        ("target_output_bias", tensors.target_output_bias),
        (
            "predictor_context_weights",
            tensors.predictor_context_weights,
        ),
        ("predictor_action_weights", tensors.predictor_action_weights),
        (
            "predictor_horizon_weights",
            tensors.predictor_horizon_weights,
        ),
        ("predictor_bias", tensors.predictor_bias),
        ("auxiliary_bias", tensors.auxiliary_bias),
        ("auxiliary_latent_weights", tensors.auxiliary_latent_weights),
        ("auxiliary_action_weights", tensors.auxiliary_action_weights),
    ];
    let tensor_bytes = named
        .into_iter()
        .map(|(name, values)| (name.to_string(), f32_bytes(&values), values.len()))
        .collect::<Vec<_>>();
    let views = tensor_bytes
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

pub fn write_jepa_checkpoint(root: &Path, model: &JepaTraceModel) -> Result<JepaCheckpointRecord> {
    match model.metadata.backend {
        BackendKind::Metal => write_jepa_mlx_array_checkpoint(root, model),
        BackendKind::Auto | BackendKind::Cpu | BackendKind::Cuda => {
            write_jepa_safetensors_checkpoint(root, model)
        }
    }
}

pub fn write_jepa_mlx_array_checkpoint(
    root: &Path,
    model: &JepaTraceModel,
) -> Result<JepaCheckpointRecord> {
    let record = JepaCheckpointRecord {
        model_id: model.metadata.model_id.clone(),
        format: "mlx_array".into(),
        path: root
            .join("jepa")
            .join("candidates")
            .join(format!("{}.mlx", model.metadata.model_id)),
    };
    if let Some(parent) = record.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let checkpoint = JepaMlxArrayCheckpoint {
        model_id: model.metadata.model_id.clone(),
        arrays: jepa_checkpoint_tensors(model),
        memory_order: "row_major".into(),
        dtype: "f32".into(),
    };
    std::fs::write(&record.path, serde_json::to_vec_pretty(&checkpoint)?)?;
    Ok(record)
}

pub fn read_jepa_safetensors_checkpoint(path: &Path) -> Result<JepaCheckpointTensors> {
    let bytes = std::fs::read(path)?;
    let tensors = safetensors::SafeTensors::deserialize(&bytes)?;
    Ok(JepaCheckpointTensors {
        context_input_weights: tensor_f32(&tensors, "context_input_weights")?,
        context_hidden_bias: tensor_f32(&tensors, "context_hidden_bias")?,
        context_output_weights: tensor_f32(&tensors, "context_output_weights")?,
        context_output_bias: tensor_f32(&tensors, "context_output_bias")?,
        action_input_weights: tensor_f32(&tensors, "action_input_weights")?,
        action_hidden_bias: tensor_f32(&tensors, "action_hidden_bias")?,
        action_output_weights: tensor_f32(&tensors, "action_output_weights")?,
        action_output_bias: tensor_f32(&tensors, "action_output_bias")?,
        target_input_weights: tensor_f32(&tensors, "target_input_weights")?,
        target_hidden_bias: tensor_f32(&tensors, "target_hidden_bias")?,
        target_output_weights: tensor_f32(&tensors, "target_output_weights")?,
        target_output_bias: tensor_f32(&tensors, "target_output_bias")?,
        predictor_context_weights: tensor_f32(&tensors, "predictor_context_weights")?,
        predictor_action_weights: tensor_f32(&tensors, "predictor_action_weights")?,
        predictor_horizon_weights: tensor_f32(&tensors, "predictor_horizon_weights")?,
        predictor_bias: tensor_f32(&tensors, "predictor_bias")?,
        auxiliary_bias: tensor_f32(&tensors, "auxiliary_bias")?,
        auxiliary_latent_weights: tensor_f32(&tensors, "auxiliary_latent_weights")?,
        auxiliary_action_weights: tensor_f32(&tensors, "auxiliary_action_weights")?,
    })
}

pub fn read_jepa_mlx_array_checkpoint(path: &Path) -> Result<JepaMlxArrayCheckpoint> {
    let content = std::fs::read_to_string(path)?;
    serde_json::from_str(&content).map_err(Into::into)
}
