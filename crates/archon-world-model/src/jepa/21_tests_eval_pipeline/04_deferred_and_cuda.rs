#[test]
#[ignore = "deferred: requires archon binary build + JEPA fixture; POSIX mechanism tested in T012"]
fn posix_background_happy_path() {
    // Stub: spawn `archon world eval-jepa <candidate> --background` via
    // std::process::Command, assert parent exits 0 in < 2s, assert run-id
    // printed to stdout, assert run record file exists, poll up to 30s for
    // status ∈ {completed, failed}.
}

// ---------------------------------------------------------------------------
// TC-JEVAL-31: Tri-backend identical gate verdict (hardware-gated)
//
// Requires MLX Metal hardware AND a CUDA device on the same machine.
// This test exists as a documented stub; run manually on qualifying hardware.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "hardware-gated: requires both MLX Metal AND CUDA device — run manually on qualifying machine"]
fn tri_backend_identical_gate_verdict() {
    // Stub:
    // 1. Load a fixed test candidate + fixed corpus rows.
    // 2. Run JepaEvalRuntime encode_batch + validate_forward_parity via:
    //    (a) CpuEvalRuntime
    //    (b) MlxEvalRuntime
    //    (c) CudaEvalRuntime
    // 3. Assert all three ParityReport.passed values are identical.
    // 4. Assert all three cosine_similarity values are within
    //    backend_parity_cosine_floor of each other (default ≥ 0.99).
    //
    // Parity tolerance per JepaEvalGateConfig::backend_parity_cosine_floor
    // (default 0.99, configurable in [learning.world_model.jepa]).
}

// ---------------------------------------------------------------------------
// TC-JEVAL-32: CUDA resolver falls back to CPU when CUDA not required
//
// The unit test `tests_eval_backends::cuda_required_but_not_compiled_fails_closed_via_resolver`
// verifies the fail-closed path (CUDA required but not compiled → Err).
//
// THIS integration test verifies the fallback path:
//   CUDA not required + not compiled → resolver returns CPU runtime with no error.
//   This matches the `tracing::warn!` + CPU fallback branch in eval_runtime.rs.
//
// On a host with the `cuda` feature compiled, this test exercises the path
// where CUDA is requested but allow_cpu_fallback=true and prefer_accelerator=false
// (i.e., CUDA is NOT required), so resolve() succeeds with CPU.
// ---------------------------------------------------------------------------

#[test]
fn cuda_resolver_falls_back_to_cpu_when_not_required() {
    // Construct metadata that has CPU as the selected_backend (not CUDA).
    let backend_execution =
        crate::jepa::JepaBackendExecutionReport::cpu(crate::BackendKind::Cpu, None, 0);
    let metadata = crate::jepa::JepaTraceModelMetadata {
        model_id: "test-cuda-fallback".to_string(),
        model_kind: "jepa_transition".to_string(),
        latent_dim: 16,
        context_window_rows: 4,
        target_window_rows: 2,
        prediction_horizons: vec![1, 3],
        mask_ratio: 0.30,
        ema_decay: 0.996,
        target_stop_gradient: true,
        backend: crate::BackendKind::Cpu,
        backend_execution,
        row_count: 0,
        example_count: 0,
        parameter_count: 0,
        created_at: chrono::Utc::now(),
    };

    // Config: training_backend = "cuda" but allow_cpu_fallback = true
    // and prefer_accelerator = false → cuda_is_required() = false.
    let config = crate::jepa::JepaEvalResolverConfig {
        training_backend: "cuda".into(),
        allow_cpu_fallback: true,
        prefer_accelerator: false,
        parity_floor: 0.99,
        min_metal_validation_examples: 0,
        min_cuda_validation_examples: 0,
    };

    // Pre-check: CUDA must NOT be required under this config.
    let cuda_required = crate::jepa::BackendRuntimeResolver::cuda_is_required(
        &metadata,
        &config,
        None, // no CLI override
    );
    assert!(
        !cuda_required,
        "cuda_is_required must be false when allow_cpu_fallback=true and prefer_accelerator=false"
    );

    // Resolve: must succeed and return a CPU runtime.
    let result = crate::jepa::BackendRuntimeResolver::resolve(&metadata, &config, None);
    assert!(
        result.is_ok(),
        "resolver must succeed (CPU fallback) when CUDA not required; got: {:?}",
        result.map(|_| ()).map_err(|e| e.to_string())
    );
    let runtime = result.unwrap();
    assert_eq!(
        runtime.backend_kind(),
        crate::jepa::JepaEvalBackendKind::Cpu,
        "resolver must return CPU backend when CUDA not required and not available"
    );
}

// ---------------------------------------------------------------------------
// TC-JEVAL-33: CLI baseline-status line format — DEFERRED
//
// This test requires full eval pipeline wiring through render_eval_jepa_with_options
// (T023 handled the CLI flag plumbing; the baseline status output path is not
// yet finalised). Deferred to post-T025 once the output format is confirmed.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "deferred: requires full eval pipeline wiring and CLI output capture — post-T025"]
fn cli_baseline_status_line_format() {
    // Stub:
    // Case 1: quick eval, Tier-0 gate fails (collapse) → output contains "Baseline: skipped"
    // Case 2: full eval, warm cache (embeddings cached) → output contains "Baseline: cached"
    // Case 3: full eval, cold cache                    → output contains "Baseline: run"
    // Case 4: eval interrupted (Paused status)          → output contains "Baseline: incomplete"
}
