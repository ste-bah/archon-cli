    #[test]
    fn cpu_jepa_backend_wraps_current_training_operations() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();
        let context_encoder = JepaTraceEncoder::new("context", config.latent_dim);
        let action_encoder = JepaTraceEncoder::new("action", config.latent_dim);
        let target_encoder = JepaTraceEncoder::ema_target_from(&context_encoder, config.ema_decay);
        let encoders = JepaEncoderSet {
            context_encoder,
            action_encoder,
            target_encoder,
        };
        let backend = CpuJepaBackend;
        let feature_batch = JepaFeatureBatch::from_examples(&examples, config.latent_dim).unwrap();

        let encoded_stage = backend.encode_batch(&encoders, &feature_batch).unwrap();
        assert_eq!(encoded_stage.execution, JepaStageExecution::native());
        let encoded = encoded_stage.value;
        let predictor = backend.fit_predictor(config.latent_dim, &encoded).unwrap();
        let transition = backend.fit_transition(config.latent_dim, &encoded).unwrap();

        assert_eq!(backend.status().selected, BackendKind::Cpu);
        assert_eq!(feature_batch.rows, examples.len());
        assert_eq!(
            feature_batch.context_features.len(),
            examples.len() * config.latent_dim
        );
        assert_eq!(encoded.len(), feature_batch.len());
        assert_eq!(predictor.execution, JepaStageExecution::native());
        assert_eq!(predictor.value.latent_dim, config.latent_dim);
        assert_eq!(transition.execution, JepaStageExecution::native());
        assert_eq!(transition.value.metadata.backend, BackendKind::Cpu);
    }

    #[test]
    fn accelerator_jepa_backend_stubs_compile_and_fail_closed() {
        let cuda = CandleCudaJepaBackend;
        let encoded = Vec::new();

        assert_eq!(cuda.probe_jepa().status.requested, BackendKind::Cuda);
        #[cfg(not(feature = "cuda"))]
        assert!(
            cuda.fit_predictor(8, &encoded)
                .unwrap_err()
                .to_string()
                .contains("native cuda JEPA tensor backend is not implemented")
        );
        #[cfg(not(all(
            feature = "mlx-metal",
            target_os = "macos",
            target_arch = "aarch64"
        )))]
        {
            let metal = MlxMetalJepaBackend;
            assert_eq!(metal.probe_jepa().status.requested, BackendKind::Metal);
            assert!(
                metal
                    .fit_predictor(8, &encoded)
                    .unwrap_err()
                    .to_string()
                    .contains("native metal JEPA tensor backend is not implemented")
            );
        }
    }

    #[cfg(feature = "candle")]
    #[test]
    fn candle_runtime_matches_cpu_jepa_runtime_on_cpu_device() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();
        let window = &examples[0].context;
        let action = &examples[0].action;

        let cpu = CpuJepaBackend
            .predict_runtime(&model, window, action)
            .unwrap()
            .value;
        let candle = candle_jepa_predict_runtime_on_device(
            &model,
            window,
            action,
            BackendKind::Cpu,
            &candle_core::Device::Cpu,
        )
        .unwrap();

        assert_eq!(candle.backend, BackendKind::Cpu);
        assert_eq!(cpu.execution_report.backend, BackendKind::Cpu);
        assert_eq!(cpu.execution_report.framework, "rust-vector");
        assert!(cpu.execution_report.native_runtime_prediction);
        assert_eq!(cpu.execution_report.host_fallback_count, 0);
        assert_eq!(cpu.execution_report.latency_ms, cpu.latency_ms);
        assert_eq!(candle.execution_report.backend, BackendKind::Cpu);
        assert_eq!(candle.execution_report.framework, "candle");
        assert!(candle.execution_report.native_runtime_prediction);
        assert_eq!(candle.execution_report.host_fallback_count, 0);
        assert_eq!(candle.execution_report.latency_ms, candle.latency_ms);
        assert_eq!(cpu.guardrail_scores, candle.guardrail_scores);
        assert!(
            cosine_error(&cpu.predicted_next_state, &candle.predicted_next_state).unwrap() < 0.001
        );
        assert_eq!(cpu.auxiliary_scores.len(), candle.auxiliary_scores.len());
        for ((left_label, left), (right_label, right)) in
            cpu.auxiliary_scores.iter().zip(&candle.auxiliary_scores)
        {
            assert_eq!(left_label, right_label);
            assert!((left - right).abs() < 0.001);
        }
    }

    #[test]
    fn requested_accelerator_with_fallback_writes_cpu_labelled_candidate() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let status =
            BackendStatus::cpu_fallback(BackendKind::Cuda, "cuda_probe_failed:not_compiled");

        let (model, outcome) =
            train_jepa_candidate_with_backend_status(&rows(), &config, status, true, None, None)
                .unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cpu);
        assert_eq!(
            outcome.metadata.backend_execution.requested_backend,
            BackendKind::Cuda
        );
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Cpu
        );
        assert_eq!(
            outcome
                .metadata
                .backend_execution
                .fallback_reason
                .as_deref(),
            Some("cuda_probe_failed:not_compiled")
        );
    }

    #[cfg(not(feature = "cuda"))]
    #[test]
    fn selected_accelerator_without_native_jepa_fails_or_relabels_cpu() {
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

        let error =
            train_jepa_candidate_with_backend_status(
                &rows(),
                &config,
                status.clone(),
                false,
                None,
                None,
            )
                .unwrap_err();
        assert!(error.to_string().contains("JepaBackendNativeStageFailed"));

        let (model, outcome) =
            train_jepa_candidate_with_backend_status(&rows(), &config, status, true, None, None)
                .unwrap();
        assert_eq!(model.metadata.backend, BackendKind::Cpu);
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Cpu
        );
        assert_eq!(
            outcome
                .metadata
                .backend_execution
                .fallback_reason
                .as_deref(),
            Some("jepa_native_backend_not_compiled:cuda")
        );
    }

    #[cfg(not(all(feature = "mlx-metal", target_os = "macos", target_arch = "aarch64")))]
    #[test]
    fn selected_metal_without_native_target_fails_or_relabels_cpu() {
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

        let error =
            train_jepa_candidate_with_backend_status(
                &rows(),
                &config,
                status.clone(),
                false,
                None,
                None,
            )
                .unwrap_err();
        assert!(error.to_string().contains("JepaBackendNativeStageFailed"));

        let (model, outcome) =
            train_jepa_candidate_with_backend_status(&rows(), &config, status, true, None, None)
                .unwrap();
        assert_eq!(model.metadata.backend, BackendKind::Cpu);
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Cpu
        );
        assert_eq!(
            outcome
                .metadata
                .backend_execution
                .fallback_reason
                .as_deref(),
            Some("jepa_native_backend_not_compiled:metal")
        );
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn cuda_jepa_training_writes_native_execution_proof_when_available() {
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
            train_jepa_candidate_with_backend_status(&rows(), &config, status, false, None, None)
                .unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cuda);
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Cuda
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
            BackendKind::Cuda
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
