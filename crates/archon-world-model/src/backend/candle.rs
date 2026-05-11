use super::{BackendKind, BackendProbeReport, BackendStatus, WorldModelBackend};
use crate::model::{CpuLatentTransitionModel, LatentTransitionExample};
use anyhow::Result;

#[derive(Debug, Clone, Default)]
pub struct CandleCpuBackend;

impl WorldModelBackend for CandleCpuBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Cpu
    }

    fn status(&self) -> BackendStatus {
        BackendStatus::cpu()
    }

    fn probe(&self) -> BackendProbeReport {
        candle_cpu_probe()
    }
}

#[cfg(feature = "candle")]
pub fn candle_cpu_probe() -> BackendProbeReport {
    match candle_core::Tensor::from_slice(&[0.0f32], 1, &candle_core::Device::Cpu)
        .and_then(|tensor| (tensor + 1.0f64)?.to_vec1::<f32>().map(|_| ()))
    {
        Ok(()) => BackendProbeReport::available(BackendKind::Cpu, "candle"),
        Err(error) => BackendProbeReport::unavailable(
            BackendKind::Cpu,
            "candle",
            true,
            true,
            false,
            format!("self_test_failed:{error}"),
        ),
    }
}

#[cfg(not(feature = "candle"))]
pub fn candle_cpu_probe() -> BackendProbeReport {
    BackendProbeReport::available(BackendKind::Cpu, "rust")
}

#[cfg(feature = "candle")]
pub fn candle_cpu_predict_next(
    model: &CpuLatentTransitionModel,
    state: &[f32],
    action: &[f32],
) -> Result<Vec<f32>> {
    candle_predict_next_on_device(model, state, action, &candle_core::Device::Cpu)
}

#[cfg(not(feature = "candle"))]
pub fn candle_cpu_predict_next(
    model: &CpuLatentTransitionModel,
    state: &[f32],
    action: &[f32],
) -> Result<Vec<f32>> {
    model.predict_next(state, action)
}

#[cfg(feature = "candle")]
pub fn candle_cpu_fit_transition_model(
    state_dim: usize,
    examples: &[LatentTransitionExample],
) -> Result<CpuLatentTransitionModel> {
    candle_fit_transition_model_on_device(
        state_dim,
        examples,
        BackendKind::Cpu,
        &candle_core::Device::Cpu,
    )
}

#[cfg(not(feature = "candle"))]
pub fn candle_cpu_fit_transition_model(
    state_dim: usize,
    examples: &[LatentTransitionExample],
) -> Result<CpuLatentTransitionModel> {
    CpuLatentTransitionModel::fit(state_dim, examples)
}

#[cfg(feature = "cuda")]
#[derive(Debug, Clone, Default)]
pub struct CandleCudaBackend;

#[cfg(feature = "cuda")]
impl WorldModelBackend for CandleCudaBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Cuda
    }

    fn status(&self) -> BackendStatus {
        BackendStatus {
            requested: BackendKind::Cuda,
            selected: BackendKind::Cuda,
            framework: "candle".into(),
            device_name: None,
            experimental: false,
            fallback_reason: None,
        }
    }

    fn probe(&self) -> BackendProbeReport {
        candle_cuda_probe()
    }
}

#[cfg(feature = "cuda")]
pub fn candle_cuda_available() -> bool {
    candle_cuda_probe().available
}

#[cfg(not(feature = "cuda"))]
pub fn candle_cuda_available() -> bool {
    false
}

#[cfg(feature = "cuda")]
pub fn candle_cuda_probe() -> BackendProbeReport {
    let device = match candle_core::Device::new_cuda(0) {
        Ok(device) => device,
        Err(error) => {
            return BackendProbeReport::unavailable(
                BackendKind::Cuda,
                "candle",
                true,
                false,
                false,
                format!("device_creation_failed:{error}"),
            );
        }
    };
    match candle_core::Tensor::from_slice(&[0.0f32], 1, &device)
        .and_then(|tensor| (tensor + 1.0f64)?.to_vec1::<f32>().map(|_| ()))
    {
        Ok(()) => BackendProbeReport::available(BackendKind::Cuda, "candle"),
        Err(error) => BackendProbeReport::unavailable(
            BackendKind::Cuda,
            "candle",
            true,
            true,
            false,
            format!("self_test_failed:{error}"),
        ),
    }
}

#[cfg(not(feature = "cuda"))]
pub fn candle_cuda_probe() -> BackendProbeReport {
    BackendProbeReport::unavailable(
        BackendKind::Cuda,
        "candle",
        false,
        false,
        false,
        "not_compiled",
    )
}

#[cfg(feature = "cuda")]
pub fn candle_cuda_predict_next(
    model: &CpuLatentTransitionModel,
    state: &[f32],
    action: &[f32],
) -> Result<Vec<f32>> {
    let device = candle_core::Device::new_cuda(0)?;
    candle_predict_next_on_device(model, state, action, &device)
}

#[cfg(not(feature = "cuda"))]
pub fn candle_cuda_predict_next(
    model: &CpuLatentTransitionModel,
    state: &[f32],
    action: &[f32],
) -> Result<Vec<f32>> {
    let _ = (model, state, action);
    anyhow::bail!("candle CUDA backend is not compiled")
}

#[cfg(feature = "cuda")]
pub fn candle_cuda_fit_transition_model(
    state_dim: usize,
    examples: &[LatentTransitionExample],
) -> Result<CpuLatentTransitionModel> {
    let device = candle_core::Device::new_cuda(0)?;
    candle_fit_transition_model_on_device(state_dim, examples, BackendKind::Cuda, &device)
}

#[cfg(not(feature = "cuda"))]
pub fn candle_cuda_fit_transition_model(
    state_dim: usize,
    examples: &[LatentTransitionExample],
) -> Result<CpuLatentTransitionModel> {
    let _ = (state_dim, examples);
    anyhow::bail!("candle CUDA backend is not compiled")
}

#[cfg(feature = "candle")]
fn candle_predict_next_on_device(
    model: &CpuLatentTransitionModel,
    state: &[f32],
    action: &[f32],
    device: &candle_core::Device,
) -> Result<Vec<f32>> {
    model.predict_next(state, action)?;
    let state = candle_core::Tensor::from_slice(state, state.len(), device)?;
    let action = candle_core::Tensor::from_slice(action, action.len(), device)?;
    let state_weights =
        candle_core::Tensor::from_slice(&model.state_weights, model.state_weights.len(), device)?;
    let action_weights =
        candle_core::Tensor::from_slice(&model.action_weights, model.action_weights.len(), device)?;
    let bias = candle_core::Tensor::from_slice(
        &model.transition_bias,
        model.transition_bias.len(),
        device,
    )?;
    let predicted =
        ((state.broadcast_mul(&state_weights)?) + (action.broadcast_mul(&action_weights)?))? + bias;
    Ok(predicted?.to_vec1::<f32>()?)
}

#[cfg(feature = "candle")]
fn candle_fit_transition_model_on_device(
    state_dim: usize,
    examples: &[LatentTransitionExample],
    backend: BackendKind,
    device: &candle_core::Device,
) -> Result<CpuLatentTransitionModel> {
    let mut model = CpuLatentTransitionModel::fit(state_dim, examples)?;
    let rows = examples.len();
    let states = candle_core::Tensor::from_vec(
        flatten_examples(examples, state_dim, |example| &example.state),
        (rows, state_dim),
        device,
    )?;
    let actions = candle_core::Tensor::from_vec(
        flatten_examples(examples, state_dim, |example| &example.action),
        (rows, state_dim),
        device,
    )?;
    let next_states = candle_core::Tensor::from_vec(
        flatten_examples(examples, state_dim, |example| &example.next_state),
        (rows, state_dim),
        device,
    )?;

    let state_means = states.mean(0)?;
    let action_means = actions.mean(0)?;
    let next_means = next_states.mean(0)?;
    let centered_states = states.broadcast_sub(&state_means)?;
    let centered_actions = actions.broadcast_sub(&action_means)?;
    let centered_next = next_states.broadcast_sub(&next_means)?;
    let state_var = (centered_states.sqr()?.mean(0)? + 1e-6f64)?;
    let action_var = (centered_actions.sqr()?.mean(0)? + 1e-6f64)?;
    let state_weights = (centered_states.broadcast_mul(&centered_next)?.mean(0)? / state_var)?;
    let action_weights = (centered_actions.broadcast_mul(&centered_next)?.mean(0)? / action_var)?;
    let weighted_state = state_weights.broadcast_mul(&state_means)?;
    let weighted_action = action_weights.broadcast_mul(&action_means)?;
    let transition_bias = next_means
        .broadcast_sub(&weighted_state)?
        .broadcast_sub(&weighted_action)?;

    model.metadata.backend = backend;
    model.state_weights = state_weights.to_vec1::<f32>()?;
    model.action_weights = action_weights.to_vec1::<f32>()?;
    model.transition_bias = transition_bias.to_vec1::<f32>()?;
    Ok(model)
}

#[cfg(feature = "candle")]
fn flatten_examples(
    examples: &[LatentTransitionExample],
    state_dim: usize,
    accessor: impl Fn(&LatentTransitionExample) -> &[f32],
) -> Vec<f32> {
    examples
        .iter()
        .flat_map(|example| {
            let values = accessor(example);
            (0..state_dim).map(move |idx| values.get(idx).copied().unwrap_or_default())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::LatentTransitionExample;
    use crate::schema::WorldLabelSet;

    #[test]
    fn candle_cpu_predicts_with_compact_model() {
        let examples = [LatentTransitionExample {
            state: vec![0.0, 0.0],
            action: vec![0.0, 0.0],
            next_state: vec![1.0, 1.0],
            labels: WorldLabelSet::default(),
        }];
        let model = CpuLatentTransitionModel::fit(2, &examples).unwrap();

        let predicted = candle_cpu_predict_next(&model, &[0.0, 0.0], &[0.0, 0.0]).unwrap();

        assert_eq!(predicted, vec![1.0, 1.0]);
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn candle_cuda_trains_and_predicts_when_available() {
        if !candle_cuda_available() {
            return;
        }
        let examples = [
            LatentTransitionExample {
                state: vec![0.0, 0.0],
                action: vec![0.0, 0.0],
                next_state: vec![1.0, 1.0],
                labels: WorldLabelSet::default(),
            },
            LatentTransitionExample {
                state: vec![1.0, 1.0],
                action: vec![0.0, 0.0],
                next_state: vec![2.0, 2.0],
                labels: WorldLabelSet::default(),
            },
        ];

        let model = candle_cuda_fit_transition_model(2, &examples).unwrap();
        let predicted = candle_cuda_predict_next(&model, &[1.0, 1.0], &[0.0, 0.0]).unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cuda);
        assert!(predicted.iter().all(|value| value.is_finite()));
    }
}
