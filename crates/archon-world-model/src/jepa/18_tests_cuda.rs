    #[cfg(feature = "cuda")]
    fn jepa_cuda_hardware_fixture() -> (
        JepaTraceModel,
        JepaTrainingOutcome,
        Vec<JepaTrainingExample>,
    ) {
        require_cuda_hardware();
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let status = BackendStatus {
            requested: BackendKind::Cuda,
            selected: BackendKind::Cuda,
            framework: "candle".into(),
            device_name: Some("cuda:0".into()),
            experimental: false,
            fallback_reason: None,
        };
        let (model, outcome) =
            train_jepa_candidate_with_backend_status(&rows(), &config, status, false).unwrap();
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();
        (model, outcome, examples)
    }

    #[cfg(feature = "cuda")]
    fn cuda_hardware_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(feature = "cuda")]
    fn require_cuda_hardware() {
        let probe = crate::backend::probe_backend(BackendKind::Cuda);
        assert!(
            probe.available,
            "CUDA hardware validation requested but CUDA probe failed: {probe:?}"
        );
    }

    #[cfg(feature = "cuda")]
    #[test]
    #[ignore = "requires CUDA hardware"]
    fn jepa_cuda_probe_passes_tensor_self_test() {
        let _guard = cuda_hardware_test_lock();
        require_cuda_hardware();
        let probe = CandleCudaJepaBackend.probe_jepa();
        assert_eq!(probe.status.selected, BackendKind::Cuda);
        assert!(probe.feature_compiled);
        assert!(probe.tensor_self_test_passed);
        assert!(probe.native_runtime_prediction);
    }

    #[cfg(feature = "cuda")]
    #[test]
    #[ignore = "requires CUDA hardware"]
    fn jepa_cuda_trains_encoder_predictor_aux_transition_native() {
        let _guard = cuda_hardware_test_lock();
        let (_, outcome, _) = jepa_cuda_hardware_fixture();
        assert!(outcome.metadata.backend_execution.native_encode);
        assert!(outcome.metadata.backend_execution.native_predictor_fit);
        assert!(outcome.metadata.backend_execution.native_auxiliary_fit);
        assert!(outcome.metadata.backend_execution.native_transition_fit);
        assert!(outcome.metadata.backend_execution.native_loss_eval);
    }

    #[cfg(feature = "cuda")]
    #[test]
    #[ignore = "requires CUDA hardware"]
    fn jepa_cuda_runtime_prediction_uses_cuda() {
        let _guard = cuda_hardware_test_lock();
        let (model, _, examples) = jepa_cuda_hardware_fixture();
        let runtime_stage = CandleCudaJepaBackend
            .predict_runtime(&model, &examples[0].context, &examples[0].action)
            .unwrap();
        assert_eq!(runtime_stage.execution, JepaStageExecution::native());
        let runtime = runtime_stage.value;
        assert_eq!(runtime.backend, BackendKind::Cuda);
        assert_eq!(runtime.execution_report.backend, BackendKind::Cuda);
        assert!(runtime.execution_report.native_runtime_prediction);
        assert_eq!(runtime.execution_report.host_fallback_count, 0);
    }

    #[cfg(feature = "cuda")]
    #[test]
    #[ignore = "requires CUDA hardware"]
    fn jepa_cuda_no_host_fallback_for_required_stages() {
        let _guard = cuda_hardware_test_lock();
        let (_, outcome, _) = jepa_cuda_hardware_fixture();
        assert_eq!(outcome.metadata.backend_execution.host_fallback_count, 0);
        assert_eq!(
            outcome.metadata.backend_execution.allowed_host_stage_count,
            0
        );
    }

    #[cfg(feature = "cuda")]
    #[test]
    #[ignore = "requires CUDA hardware"]
    fn jepa_cuda_candidate_metadata_records_cuda() {
        let _guard = cuda_hardware_test_lock();
        let (model, outcome, _) = jepa_cuda_hardware_fixture();
        assert_eq!(model.metadata.backend, BackendKind::Cuda);
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Cuda
        );
        assert_eq!(outcome.metadata.backend_execution.framework, "candle");
    }

    #[cfg(feature = "cuda")]
    #[test]
    #[ignore = "requires CUDA hardware"]
    fn jepa_cuda_promotion_requires_backend_proof() {
        let _guard = cuda_hardware_test_lock();
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.metadata.backend = BackendKind::Cuda;

        assert!(validate_jepa_backend_execution(&model.metadata).is_err());
        assert!(!jepa_backend_promotion_gate(&model.metadata, 1, 1));
    }

    #[cfg(feature = "cuda")]
    #[test]
    #[ignore = "requires CUDA hardware"]
    fn jepa_cuda_parity_with_cpu_fixture() {
        let _guard = cuda_hardware_test_lock();
        let (model, _, examples) = jepa_cuda_hardware_fixture();
        assert!(jepa_backend_forward_parity_gate(
            &model,
            &examples[0].context,
            &examples[0].action,
            0.99
        ));
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn cuda_jepa_training_can_meet_hardware_validation_floor_when_available() {
        if !crate::backend::cuda_runtime_available() {
            return;
        }
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let status = BackendStatus {
            requested: BackendKind::Cuda,
            selected: BackendKind::Cuda,
            framework: "candle".into(),
            device_name: Some("cuda:0".into()),
            experimental: false,
            fallback_reason: None,
        };

        let (model, outcome) =
            train_jepa_candidate_with_backend_status(&validation_rows(520), &config, status, false)
                .unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cuda);
        assert!(outcome.metadata.backend_execution.validation_example_count >= 512);
        assert_eq!(outcome.metadata.backend_execution.host_fallback_count, 0);
        assert!(jepa_backend_promotion_gate(&model.metadata, 512, 512));
    }

