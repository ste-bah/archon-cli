use super::{BackendKind, BackendProbeReport};
#[cfg(feature = "mlx-metal")]
use super::{BackendStatus, WorldModelBackend};
use crate::model::{CpuLatentTransitionModel, LatentTransitionExample};
use anyhow::Result;

#[cfg(feature = "mlx-metal")]
#[derive(Debug, Clone, Default)]
pub struct MlxMetalBackend;

#[cfg(feature = "mlx-metal")]
impl WorldModelBackend for MlxMetalBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Metal
    }

    fn status(&self) -> BackendStatus {
        BackendStatus {
            requested: BackendKind::Metal,
            selected: BackendKind::Metal,
            framework: "mlx-rs".into(),
            device_name: None,
            experimental: true,
            fallback_reason: None,
        }
    }

    fn probe(&self) -> BackendProbeReport {
        mlx_metal_probe()
    }
}

pub fn mlx_metal_available() -> bool {
    mlx_metal_probe().available
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
pub fn mlx_metal_probe() -> BackendProbeReport {
    use mlx_rs::{Array, Device};

    Device::set_default(&Device::gpu());
    let self_test: Result<()> = (|| {
        let result = Array::from_slice(&[0.0f32], &[1]).add(&Array::from_f32(1.0))?;
        result.eval()?;
        result.try_as_slice::<f32>()?;
        Ok(())
    })();

    match self_test {
        Ok(()) => BackendProbeReport::available(BackendKind::Metal, "mlx-rs"),
        Err(error) => BackendProbeReport::unavailable(
            BackendKind::Metal,
            "mlx-rs",
            true,
            true,
            false,
            format!("self_test_failed:{error}"),
        ),
    }
}

#[cfg(all(
    feature = "mlx-metal",
    not(all(target_os = "macos", target_arch = "aarch64"))
))]
pub fn mlx_metal_probe() -> BackendProbeReport {
    BackendProbeReport::unavailable(
        BackendKind::Metal,
        "mlx-rs",
        true,
        false,
        false,
        "unsupported_target",
    )
}

#[cfg(not(feature = "mlx-metal"))]
pub fn mlx_metal_probe() -> BackendProbeReport {
    BackendProbeReport::unavailable(
        BackendKind::Metal,
        "mlx-rs",
        false,
        false,
        false,
        "not_compiled",
    )
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
pub fn mlx_metal_predict_next(
    model: &CpuLatentTransitionModel,
    state: &[f32],
    action: &[f32],
) -> Result<Vec<f32>> {
    use mlx_rs::{Array, Device};

    model.predict_next(state, action)?;
    Device::set_default(&Device::gpu());
    let state = Array::from_slice(state, &[state.len() as i32]);
    let action = Array::from_slice(action, &[action.len() as i32]);
    let state_weights =
        Array::from_slice(&model.state_weights, &[model.state_weights.len() as i32]);
    let action_weights =
        Array::from_slice(&model.action_weights, &[model.action_weights.len() as i32]);
    let bias = Array::from_slice(
        &model.transition_bias,
        &[model.transition_bias.len() as i32],
    );
    let predicted = state
        .multiply(&state_weights)?
        .add(&action.multiply(&action_weights)?)?
        .add(&bias)?;
    predicted.eval()?;
    Ok(predicted.try_as_slice::<f32>()?.to_vec())
}

#[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
pub fn mlx_metal_predict_next(
    model: &CpuLatentTransitionModel,
    state: &[f32],
    action: &[f32],
) -> Result<Vec<f32>> {
    let _ = (model, state, action);
    anyhow::bail!("MLX Metal backend is not compiled for this target")
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
pub fn mlx_metal_fit_transition_model(
    state_dim: usize,
    examples: &[LatentTransitionExample],
) -> Result<CpuLatentTransitionModel> {
    use mlx_rs::{Array, Device};

    if examples.is_empty() {
        anyhow::bail!("at least one transition example is required");
    }
    Device::set_default(&Device::gpu());
    let rows = examples.len();
    let shape = &[rows as i32, state_dim as i32];
    let states = Array::from_slice(
        &flatten_examples(examples, state_dim, |example| &example.state),
        shape,
    );
    let actions = Array::from_slice(
        &flatten_examples(examples, state_dim, |example| &example.action),
        shape,
    );
    let next_states = Array::from_slice(
        &flatten_examples(examples, state_dim, |example| &example.next_state),
        shape,
    );
    let state_means = states.mean_axis(0, None)?;
    let action_means = actions.mean_axis(0, None)?;
    let next_means = next_states.mean_axis(0, None)?;
    let centered_states = states.subtract(&state_means)?;
    let centered_actions = actions.subtract(&action_means)?;
    let centered_next = next_states.subtract(&next_means)?;
    let state_var = centered_states
        .square()?
        .mean_axis(0, None)?
        .add(&Array::from_f32(1e-6))?;
    let action_var = centered_actions
        .square()?
        .mean_axis(0, None)?
        .add(&Array::from_f32(1e-6))?;
    let state_weights = centered_states
        .multiply(&centered_next)?
        .mean_axis(0, None)?
        .divide(&state_var)?;
    let action_weights = centered_actions
        .multiply(&centered_next)?
        .mean_axis(0, None)?
        .divide(&action_var)?;
    let transition_bias = next_means
        .subtract(&state_weights.multiply(&state_means)?)?
        .subtract(&action_weights.multiply(&action_means)?)?;
    let mean_delta = next_means.subtract(&state_means)?;

    CpuLatentTransitionModel::from_fitted_transition_parts(
        state_dim,
        BackendKind::Metal,
        rows as u64,
        state_weights.try_as_slice::<f32>()?.to_vec(),
        action_weights.try_as_slice::<f32>()?.to_vec(),
        transition_bias.try_as_slice::<f32>()?.to_vec(),
        mean_delta.try_as_slice::<f32>()?.to_vec(),
        examples,
    )
}

#[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
pub fn mlx_metal_fit_transition_model(
    state_dim: usize,
    examples: &[LatentTransitionExample],
) -> Result<CpuLatentTransitionModel> {
    let _ = (state_dim, examples);
    anyhow::bail!("MLX Metal backend is not compiled for this target")
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
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
