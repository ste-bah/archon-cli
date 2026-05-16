    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "requires Apple Silicon MLX Metal"]
    fn jepa_mlx_training_writes_native_execution_proof_when_available() {
        require_mlx_hardware();
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let status = BackendStatus {
            requested: BackendKind::Metal,
            selected: BackendKind::Metal,
            framework: "mlx-rs".into(),
            device_name: Some("metal:0".into()),
            experimental: true,
            fallback_reason: None,
        };

        let (model, outcome) =
            train_jepa_candidate_with_backend_status(&rows(), &config, status, false).unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Metal);
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Metal
        );
        assert_eq!(outcome.metadata.backend_execution.host_fallback_count, 0);
        assert!(outcome.metadata.backend_execution.native_encode);
        assert!(outcome.metadata.backend_execution.native_predictor_fit);
        assert!(outcome.metadata.backend_execution.native_auxiliary_fit);
        assert!(outcome.metadata.backend_execution.native_transition_fit);
        assert!(outcome.metadata.backend_execution.native_loss_eval);
        assert_eq!(
            outcome.metadata.backend_execution.native_runtime_prediction,
            Some(true)
        );
        assert!(
            outcome
                .metadata
                .backend_execution
                .hardware_validation_captured_at
                .is_some()
        );
        assert_eq!(
            model.transition_model.as_ref().unwrap().metadata.backend,
            BackendKind::Metal
        );
        validate_jepa_backend_execution(&model.metadata).unwrap();
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();
        assert!(jepa_backend_forward_parity_gate(
            &model,
            &examples[0].context,
            &examples[0].action,
            0.99
        ));
    }

    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    fn jepa_mlx_hardware_fixture() -> (
        JepaTraceModel,
        JepaTrainingOutcome,
        Vec<JepaTrainingExample>,
    ) {
        require_mlx_hardware();
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let status = BackendStatus {
            requested: BackendKind::Metal,
            selected: BackendKind::Metal,
            framework: "mlx-rs".into(),
            device_name: Some("metal:0".into()),
            experimental: true,
            fallback_reason: None,
        };
        let (model, outcome) =
            train_jepa_candidate_with_backend_status(&rows(), &config, status, false).unwrap();
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();
        (model, outcome, examples)
    }

    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    fn require_mlx_hardware() {
        let probe = crate::backend::probe_backend(BackendKind::Metal);
        assert!(
            probe.available,
            "MLX Metal hardware validation requested but Metal probe failed: {probe:?}"
        );
    }

    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "requires Apple Silicon MLX Metal"]
    fn jepa_mlx_probe_passes_tensor_self_test() {
        require_mlx_hardware();
        let probe = MlxMetalJepaBackend.probe_jepa();
        assert_eq!(probe.status.selected, BackendKind::Metal);
        assert!(probe.feature_compiled);
        assert!(probe.tensor_self_test_passed);
        assert!(probe.native_runtime_prediction);
    }

    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "requires Apple Silicon MLX Metal"]
    fn jepa_mlx_trains_encoder_predictor_aux_transition_native() {
        let (_, outcome, _) = jepa_mlx_hardware_fixture();
        assert!(outcome.metadata.backend_execution.native_encode);
        assert!(outcome.metadata.backend_execution.native_predictor_fit);
        assert!(outcome.metadata.backend_execution.native_auxiliary_fit);
        assert!(outcome.metadata.backend_execution.native_transition_fit);
        assert!(outcome.metadata.backend_execution.native_loss_eval);
    }

    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "requires Apple Silicon MLX Metal"]
    fn jepa_mlx_runtime_prediction_uses_metal() {
        let (model, _, examples) = jepa_mlx_hardware_fixture();
        let runtime_stage = MlxMetalJepaBackend
            .predict_runtime(&model, &examples[0].context, &examples[0].action)
            .unwrap();
        assert_eq!(runtime_stage.execution, JepaStageExecution::native());
        let runtime = runtime_stage.value;
        assert_eq!(runtime.backend, BackendKind::Metal);
        assert_eq!(runtime.execution_report.backend, BackendKind::Metal);
        assert!(runtime.execution_report.native_runtime_prediction);
        assert_eq!(runtime.execution_report.host_fallback_count, 0);
    }

    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "requires Apple Silicon MLX Metal"]
    fn jepa_mlx_no_host_fallback_for_required_stages() {
        let (_, outcome, _) = jepa_mlx_hardware_fixture();
        assert_eq!(outcome.metadata.backend_execution.host_fallback_count, 0);
        assert_eq!(
            outcome.metadata.backend_execution.allowed_host_stage_count,
            0
        );
    }

    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "requires Apple Silicon MLX Metal"]
    fn jepa_mlx_candidate_metadata_records_metal() {
        let (model, outcome, _) = jepa_mlx_hardware_fixture();
        assert_eq!(model.metadata.backend, BackendKind::Metal);
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Metal
        );
        assert_eq!(outcome.metadata.backend_execution.framework, "mlx-rs");
    }

    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "requires Apple Silicon MLX Metal"]
    fn jepa_mlx_promotion_requires_backend_proof() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.metadata.backend = BackendKind::Metal;

        assert!(validate_jepa_backend_execution(&model.metadata).is_err());
        assert!(!jepa_backend_promotion_gate(&model.metadata, 1, 1));
    }

    #[cfg(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "requires Apple Silicon MLX Metal"]
    fn jepa_mlx_parity_with_cpu_fixture() {
        let (model, _, examples) = jepa_mlx_hardware_fixture();
        assert!(jepa_backend_forward_parity_gate(
            &model,
            &examples[0].context,
            &examples[0].action,
            0.99
        ));
    }
