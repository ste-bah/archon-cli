pub fn train_jepa_candidate(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    train_jepa_candidate_with_backend(rows, config, BackendKind::Cpu, true)
}

pub fn train_jepa_candidate_with_backend(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    requested_backend: BackendKind,
    allow_cpu_fallback: bool,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    train_jepa_candidate_with_backend_controlled(
        rows,
        config,
        requested_backend,
        allow_cpu_fallback,
        None,
    )
}

pub fn train_jepa_candidate_with_backend_controlled(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    requested_backend: BackendKind,
    allow_cpu_fallback: bool,
    should_stop: Option<&dyn Fn() -> bool>,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    train_jepa_candidate_with_backend_observed(
        rows,
        config,
        requested_backend,
        allow_cpu_fallback,
        should_stop,
        None,
    )
}

pub fn train_jepa_candidate_with_backend_observed(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    requested_backend: BackendKind,
    allow_cpu_fallback: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    let status = crate::backend::select_runtime_backend(requested_backend, allow_cpu_fallback);
    emit_jepa_progress(
        progress,
        "jepa_backend_selected",
        &format!(
            "requested={} selected={} fallback={}",
            status.requested,
            status.selected,
            status.fallback_reason.as_deref().unwrap_or("none")
        ),
    );
    train_jepa_candidate_with_backend_status(
        rows,
        config,
        status,
        allow_cpu_fallback,
        should_stop,
        progress,
    )
}

#[cfg(all(test, feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
static JEPA_TRAINING_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn train_jepa_candidate_with_backend_status(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    status: BackendStatus,
    allow_cpu_fallback: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    #[cfg(all(test, feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    let _jepa_training_test_lock = JEPA_TRAINING_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if status.selected == BackendKind::Cpu {
        return train_jepa_candidate_cpu(rows, config, status, should_stop, progress);
    }
    if !allow_cpu_fallback
        && let Some(reason) = status.fallback_reason.as_deref()
    {
        bail!(
            "JepaBackendProbeFailed: requested JEPA backend {} unavailable: {reason}",
            status.selected
        );
    }
    if status.selected == BackendKind::Cuda {
        return train_cuda_or_fallback(rows, config, status, allow_cpu_fallback, should_stop, progress);
    }
    if status.selected == BackendKind::Metal {
        return train_metal_or_fallback(rows, config, status, allow_cpu_fallback, should_stop, progress);
    }
    if allow_cpu_fallback {
        let fallback = BackendStatus::cpu_fallback(
            status.requested,
            format!("jepa_native_backend_not_implemented:{}", status.selected),
        );
        emit_jepa_progress(progress, "jepa_backend_fallback", &format_backend_status(&fallback));
        return train_jepa_candidate_cpu(rows, config, fallback, should_stop, progress);
    }
    bail!(
        "JepaBackendNativeStageFailed: native JEPA backend for {} is not implemented; refusing to write an accelerator-labelled candidate",
        status.selected
    );
}

#[cfg(feature = "cuda")]
fn train_cuda_or_fallback(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    status: BackendStatus,
    allow_cpu_fallback: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    match train_jepa_candidate_with_tensor_backend(
        rows,
        config,
        status.clone(),
        CandleCudaJepaBackend,
        should_stop,
        progress,
    ) {
        Ok(candidate) => Ok(candidate),
        Err(error) if allow_cpu_fallback => {
            let fallback = BackendStatus::cpu_fallback(
                status.requested,
                format!("jepa_native_backend_failed:{}:{error}", status.selected),
            );
            emit_jepa_progress(progress, "jepa_backend_fallback", &format_backend_status(&fallback));
            train_jepa_candidate_cpu(rows, config, fallback, should_stop, progress)
        }
        Err(error) => bail!(
            "JepaBackendNativeStageFailed: native JEPA backend for {} failed; refusing to write an accelerator-labelled candidate: {error}",
            status.selected
        ),
    }
}

#[cfg(not(feature = "cuda"))]
fn train_cuda_or_fallback(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    status: BackendStatus,
    allow_cpu_fallback: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    train_uncompiled_backend_or_fallback(
        rows,
        config,
        status,
        allow_cpu_fallback,
        should_stop,
        progress,
        "cuda",
    )
}

#[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
fn train_metal_or_fallback(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    status: BackendStatus,
    allow_cpu_fallback: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    match train_jepa_candidate_with_tensor_backend(
        rows,
        config,
        status.clone(),
        MlxMetalJepaBackend,
        should_stop,
        progress,
    ) {
        Ok(candidate) => Ok(candidate),
        Err(error) if allow_cpu_fallback => {
            let fallback = BackendStatus::cpu_fallback(
                status.requested,
                format!("jepa_native_backend_failed:{}:{error}", status.selected),
            );
            emit_jepa_progress(progress, "jepa_backend_fallback", &format_backend_status(&fallback));
            train_jepa_candidate_cpu(rows, config, fallback, should_stop, progress)
        }
        Err(error) => bail!(
            "JepaBackendNativeStageFailed: native JEPA backend for {} failed; refusing to write an accelerator-labelled candidate: {error}",
            status.selected
        ),
    }
}

#[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
fn train_metal_or_fallback(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    status: BackendStatus,
    allow_cpu_fallback: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    train_uncompiled_backend_or_fallback(
        rows,
        config,
        status,
        allow_cpu_fallback,
        should_stop,
        progress,
        "metal",
    )
}

fn train_uncompiled_backend_or_fallback(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    status: BackendStatus,
    allow_cpu_fallback: bool,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
    backend_name: &str,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    if !allow_cpu_fallback {
        bail!(
            "JepaBackendNativeStageFailed: native JEPA backend for {} is not compiled; refusing to write an accelerator-labelled candidate",
            status.selected
        );
    }
    let fallback = BackendStatus::cpu_fallback(
        status.requested,
        format!("jepa_native_backend_not_compiled:{backend_name}"),
    );
    emit_jepa_progress(progress, "jepa_backend_fallback", &format_backend_status(&fallback));
    train_jepa_candidate_cpu(rows, config, fallback, should_stop, progress)
}

fn train_jepa_candidate_cpu(
    rows: &[WorldTraceRow],
    config: &JepaTrainingConfig,
    backend_status: BackendStatus,
    should_stop: Option<&dyn Fn() -> bool>,
    progress: Option<&dyn Fn(&str, &str)>,
) -> Result<(JepaTraceModel, JepaTrainingOutcome)> {
    train_jepa_candidate_with_tensor_backend(
        rows,
        config,
        backend_status,
        CpuJepaBackend,
        should_stop,
        progress,
    )
}

fn emit_jepa_progress(progress: Option<&dyn Fn(&str, &str)>, stage: &str, detail: &str) {
    if let Some(progress) = progress {
        progress(stage, detail);
    }
}

fn format_backend_status(status: &BackendStatus) -> String {
    format!(
        "requested={} selected={} framework={} device={} fallback={}",
        status.requested,
        status.selected,
        status.framework,
        status.device_name.as_deref().unwrap_or("unknown"),
        status.fallback_reason.as_deref().unwrap_or("none")
    )
}
