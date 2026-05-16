pub mod bridge;
pub mod candle;
pub mod mlx;

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::model::CpuLatentTransitionModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    Auto,
    Cpu,
    Metal,
    Cuda,
}

impl Default for BackendKind {
    fn default() -> Self {
        Self::Auto
    }
}

impl fmt::Display for BackendKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Metal => "metal",
            Self::Cuda => "cuda",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendStatus {
    pub requested: BackendKind,
    pub selected: BackendKind,
    pub framework: String,
    pub device_name: Option<String>,
    pub experimental: bool,
    pub fallback_reason: Option<String>,
}

impl BackendStatus {
    pub fn cpu() -> Self {
        Self {
            requested: BackendKind::Cpu,
            selected: BackendKind::Cpu,
            framework: "candle".into(),
            device_name: Some("cpu".into()),
            experimental: false,
            fallback_reason: None,
        }
    }

    pub fn cpu_fallback(requested: BackendKind, reason: impl Into<String>) -> Self {
        Self {
            requested,
            selected: BackendKind::Cpu,
            framework: "candle".into(),
            device_name: Some("cpu".into()),
            experimental: false,
            fallback_reason: Some(reason.into()),
        }
    }
}

pub trait WorldModelBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn status(&self) -> BackendStatus;
    fn probe(&self) -> BackendProbeReport;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendProbeReport {
    pub backend: BackendKind,
    pub framework: String,
    pub compiled: bool,
    pub device_created: bool,
    pub tensor_self_test_passed: bool,
    pub available: bool,
    pub reason: Option<String>,
}

impl BackendProbeReport {
    pub fn available(backend: BackendKind, framework: impl Into<String>) -> Self {
        Self {
            backend,
            framework: framework.into(),
            compiled: true,
            device_created: true,
            tensor_self_test_passed: true,
            available: true,
            reason: None,
        }
    }

    pub fn unavailable(
        backend: BackendKind,
        framework: impl Into<String>,
        compiled: bool,
        device_created: bool,
        tensor_self_test_passed: bool,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            backend,
            framework: framework.into(),
            compiled,
            device_created,
            tensor_self_test_passed,
            available: false,
            reason: Some(reason.into()),
        }
    }
}

pub fn select_backend_status(
    requested: BackendKind,
    cuda_available: bool,
    metal_available: bool,
    allow_cpu_fallback: bool,
) -> BackendStatus {
    match requested {
        BackendKind::Cpu => BackendStatus::cpu(),
        BackendKind::Cuda if cuda_available => BackendStatus {
            requested,
            selected: BackendKind::Cuda,
            framework: "candle".into(),
            device_name: None,
            experimental: false,
            fallback_reason: None,
        },
        BackendKind::Metal if metal_available => BackendStatus {
            requested,
            selected: BackendKind::Metal,
            framework: "mlx-rs".into(),
            device_name: None,
            experimental: true,
            fallback_reason: None,
        },
        BackendKind::Auto if cuda_available => {
            select_backend_status(BackendKind::Cuda, true, false, true)
        }
        BackendKind::Auto if metal_available => {
            select_backend_status(BackendKind::Metal, false, true, true)
        }
        BackendKind::Metal if allow_cpu_fallback => {
            BackendStatus::cpu_fallback(requested, "metal_backend_failed")
        }
        _ if allow_cpu_fallback => {
            BackendStatus::cpu_fallback(requested, "accelerator_unavailable")
        }
        _ => BackendStatus {
            requested,
            selected: requested,
            framework: "unavailable".into(),
            device_name: None,
            experimental: false,
            fallback_reason: Some("backend_unavailable".into()),
        },
    }
}

pub fn cuda_runtime_available() -> bool {
    probe_backend(BackendKind::Cuda).available
}

pub fn metal_runtime_available() -> bool {
    probe_backend(BackendKind::Metal).available
}

pub fn probe_backend(backend: BackendKind) -> BackendProbeReport {
    match backend {
        BackendKind::Cpu | BackendKind::Auto => candle::candle_cpu_probe(),
        BackendKind::Cuda => candle::candle_cuda_probe(),
        BackendKind::Metal => mlx::mlx_metal_probe(),
    }
}

pub fn select_runtime_backend(requested: BackendKind, allow_cpu_fallback: bool) -> BackendStatus {
    let cuda = probe_backend(BackendKind::Cuda);
    let metal = probe_backend(BackendKind::Metal);
    select_backend_status_from_probes(requested, &cuda, &metal, allow_cpu_fallback)
}

pub fn select_backend_status_from_probes(
    requested: BackendKind,
    cuda: &BackendProbeReport,
    metal: &BackendProbeReport,
    allow_cpu_fallback: bool,
) -> BackendStatus {
    match requested {
        BackendKind::Cpu => BackendStatus::cpu(),
        BackendKind::Cuda if cuda.available => BackendStatus {
            requested,
            selected: BackendKind::Cuda,
            framework: cuda.framework.clone(),
            device_name: None,
            experimental: false,
            fallback_reason: None,
        },
        BackendKind::Metal if metal.available => BackendStatus {
            requested,
            selected: BackendKind::Metal,
            framework: metal.framework.clone(),
            device_name: None,
            experimental: true,
            fallback_reason: None,
        },
        BackendKind::Auto if cuda.available => {
            select_backend_status_from_probes(BackendKind::Cuda, cuda, metal, true)
        }
        BackendKind::Auto if metal.available => {
            select_backend_status_from_probes(BackendKind::Metal, cuda, metal, true)
        }
        BackendKind::Cuda if allow_cpu_fallback => {
            BackendStatus::cpu_fallback(requested, probe_fallback_reason("cuda", cuda))
        }
        BackendKind::Metal if allow_cpu_fallback => {
            BackendStatus::cpu_fallback(requested, probe_fallback_reason("metal", metal))
        }
        BackendKind::Auto if allow_cpu_fallback => BackendStatus::cpu_fallback(
            requested,
            format!(
                "accelerator_unavailable:cuda={};metal={}",
                cuda.reason.as_deref().unwrap_or("unknown"),
                metal.reason.as_deref().unwrap_or("unknown")
            ),
        ),
        _ => BackendStatus {
            requested,
            selected: requested,
            framework: "unavailable".into(),
            device_name: None,
            experimental: false,
            fallback_reason: Some("backend_unavailable".into()),
        },
    }
}

fn probe_fallback_reason(prefix: &str, probe: &BackendProbeReport) -> String {
    format!(
        "{prefix}_probe_failed:{}",
        probe.reason.as_deref().unwrap_or("unknown")
    )
}

pub fn predict_next_with_backend(
    model: &CpuLatentTransitionModel,
    state: &[f32],
    action: &[f32],
    backend: BackendKind,
) -> anyhow::Result<Vec<f32>> {
    match backend {
        BackendKind::Cuda => candle::candle_cuda_predict_next(model, state, action),
        BackendKind::Metal => mlx::mlx_metal_predict_next(model, state, action),
        BackendKind::Auto | BackendKind::Cpu => {
            candle::candle_cpu_predict_next(model, state, action)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metal_backend_can_fallback_to_cpu() {
        let status = select_backend_status(BackendKind::Metal, false, false, true);

        assert_eq!(status.selected, BackendKind::Cpu);
        assert_eq!(
            status.fallback_reason.as_deref(),
            Some("metal_backend_failed")
        );
    }

    #[test]
    fn auto_prefers_available_accelerator() {
        let status = select_backend_status(BackendKind::Auto, true, false, true);

        assert_eq!(status.selected, BackendKind::Cuda);
        assert_eq!(status.framework, "candle");
    }

    #[test]
    fn runtime_selection_uses_tensor_self_test_probe() {
        let cuda = BackendProbeReport::unavailable(
            BackendKind::Cuda,
            "candle",
            true,
            true,
            false,
            "self_test_failed:unsupported_ptx",
        );
        let metal = BackendProbeReport::unavailable(
            BackendKind::Metal,
            "mlx-rs",
            false,
            false,
            false,
            "not_compiled",
        );

        let status = select_backend_status_from_probes(BackendKind::Cuda, &cuda, &metal, true);

        assert_eq!(status.selected, BackendKind::Cpu);
        assert!(
            status
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("self_test_failed")
        );
    }

    #[test]
    fn accelerator_transition_fit_paths_do_not_cpu_fit_first() {
        let candle = include_str!("candle.rs");
        let candle_body = function_source(candle, "fn candle_fit_transition_model_on_device");
        assert!(
            !candle_body.contains("CpuLatentTransitionModel::fit"),
            "Candle accelerator transition fitting must not call CPU fit first"
        );

        let mlx = include_str!("mlx.rs");
        let mlx_body = function_source(mlx, "pub fn mlx_metal_fit_transition_model");
        assert!(
            !mlx_body.contains("CpuLatentTransitionModel::fit"),
            "MLX accelerator transition fitting must not call CPU fit first"
        );
    }

    fn function_source<'a>(source: &'a str, marker: &str) -> &'a str {
        let start = source
            .find(marker)
            .expect("backend source should contain marker");
        let rest = &source[start..];
        let end = rest.find("\n#[cfg").unwrap_or(rest.len());
        &rest[..end]
    }
}
