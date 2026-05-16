    #[test]
    fn strict_probe_failure_uses_typed_jepa_reason() {
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
            framework: "unavailable".into(),
            device_name: None,
            experimental: false,
            fallback_reason: Some("cuda_probe_failed:not_compiled".into()),
        };

        let error =
            train_jepa_candidate_with_backend_status(&rows(), &config, status, false).unwrap_err();

        assert!(error.to_string().contains("JepaBackendProbeFailed"));
    }

    #[test]
    fn cuda_metadata_without_native_execution_proof_is_rejected() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.metadata.backend = BackendKind::Cuda;

        let error = validate_jepa_backend_execution(&model.metadata).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("JepaBackendHostFallbackRejected")
        );
    }

    #[test]
    fn accelerator_promotion_gate_requires_hardware_validation_report() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.metadata.backend = BackendKind::Cuda;
        model.metadata.backend_execution = JepaBackendExecutionReport {
            requested_backend: BackendKind::Cuda,
            selected_backend: BackendKind::Cuda,
            framework: "candle".into(),
            device_name: Some("cuda:0".into()),
            commit_sha: "abc123".into(),
            feature_compiled: true,
            tensor_self_test_passed: true,
            hardware_validation_captured_at: None,
            validation_example_count: 512,
            native_encode: true,
            native_predictor_fit: true,
            native_auxiliary_fit: true,
            native_transition_fit: true,
            native_loss_eval: true,
            native_runtime_prediction: Some(true),
            host_fallback_count: 0,
            allowed_host_stage_count: 0,
            fallback_reason: None,
        };

        assert!(validate_jepa_backend_execution(&model.metadata).is_ok());
        assert!(!jepa_backend_promotion_gate(&model.metadata, 512, 512));
        assert_eq!(
            jepa_backend_promotion_gate_failure(&model.metadata, 512, 512),
            Some("JepaBackendHardwareValidationMissing")
        );

        model
            .metadata
            .backend_execution
            .hardware_validation_captured_at = Some(Utc::now());

        assert!(jepa_backend_promotion_gate(&model.metadata, 512, 512));
        assert_eq!(
            jepa_backend_promotion_gate_failure(&model.metadata, 512, 512),
            None
        );
    }

    #[test]
    fn accelerator_execution_proof_requires_native_runtime_prediction() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.metadata.backend = BackendKind::Cuda;
        model.metadata.backend_execution = JepaBackendExecutionReport {
            requested_backend: BackendKind::Cuda,
            selected_backend: BackendKind::Cuda,
            framework: "candle".into(),
            device_name: Some("cuda:0".into()),
            commit_sha: "abc123".into(),
            feature_compiled: true,
            tensor_self_test_passed: true,
            hardware_validation_captured_at: Some(Utc::now()),
            validation_example_count: 512,
            native_encode: true,
            native_predictor_fit: true,
            native_auxiliary_fit: true,
            native_transition_fit: true,
            native_loss_eval: true,
            native_runtime_prediction: Some(false),
            host_fallback_count: 0,
            allowed_host_stage_count: 0,
            fallback_reason: None,
        };

        let error = validate_jepa_backend_execution(&model.metadata).unwrap_err();

        assert!(error.to_string().contains("JepaBackendNativeStageFailed"));
        assert_eq!(
            jepa_backend_promotion_gate_failure(&model.metadata, 512, 512),
            Some("JepaBackendNativeStageFailed")
        );
    }

    #[test]
    fn target_encoder_is_ema_of_context_encoder() {
        let context = JepaTraceEncoder::new("context", 8);
        let initialized_target = JepaTraceEncoder::new("target", 8);
        let target = JepaTraceEncoder::ema_target_from(&context, 0.5);

        assert_eq!(target.role, "target");
        let expected = 0.5 * initialized_target.input_weights[0] + 0.5 * context.input_weights[0];
        assert!((target.input_weights[0] - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn collapse_gate_rejects_constant_latents() {
        let latents = vec![vec![0.5; 8]; 4];

        let report = evaluate_representation_collapse(&latents, 0.05, 0.50).unwrap();

        assert!(!report.passes);
        assert_eq!(report.mean_latent_std, 0.0);
        assert_eq!(report.effective_rank_ratio, 0.0);
    }

    #[test]
    fn collapse_gate_rejects_rank_one_latents_with_nonzero_std() {
        let direction = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
        let latents = [-3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0]
            .into_iter()
            .map(|scale| {
                direction
                    .iter()
                    .map(|component| scale * component)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let report = evaluate_representation_collapse(&latents, 0.05, 0.50).unwrap();

        assert!(report.mean_latent_std >= 0.05);
        assert!(report.effective_rank_ratio < 0.50);
        assert!(!report.passes);
    }

    #[test]
    fn collapse_gate_accepts_full_rank_latents() {
        let mut latents = Vec::new();
        for idx in 0..8 {
            let mut positive = vec![0.0; 8];
            positive[idx] = 3.0;
            latents.push(positive);

            let mut negative = vec![0.0; 8];
            negative[idx] = -3.0;
            latents.push(negative);
        }

        let report = evaluate_representation_collapse(&latents, 0.05, 0.50).unwrap();

        assert!(report.mean_latent_std >= 0.05);
        assert!(report.effective_rank_ratio >= 0.99);
        assert!(report.passes);
    }

    #[test]
    fn jepa_module_keeps_encoder_path_free_of_embedding_adapters() {
        let sources = [
            include_str!("../jepa.rs"),
            include_str!("00_config_metadata.rs"),
            include_str!("01_model.rs"),
            include_str!("02_records_backend_types.rs"),
            include_str!("03_backend_impls.rs"),
            include_str!("04_candle_runtime.rs"),
            include_str!("05_candle_training.rs"),
            include_str!("06_mlx_runtime.rs"),
            include_str!("07_mlx_training.rs"),
            include_str!("08_examples_eval.rs"),
            include_str!("09_training_runtime.rs"),
            include_str!("10_checkpoint_io.rs"),
            include_str!("11_mask_encode_loss.rs"),
            include_str!("12_features.rs"),
            include_str!("13_aux_math_utils.rs"),
        ];
        let forbidden_fragments = [
            ("Memory", "EmbeddingAdapter"),
            ("World", "EmbeddingAdapter"),
            ("Embedding", "Request"),
            ("Embedding", "Vector"),
            ("DeterministicHash", "EmbeddingAdapter"),
            ("local_", "fastembed"),
            ("OpenAI", "Embedding"),
            ("Fast", "Embed"),
            (".", "embed("),
        ];

        for (left, right) in forbidden_fragments {
            let forbidden = format!("{left}{right}");
            for source in sources {
                assert!(
                    !source.contains(&forbidden),
                    "JEPA module must not reference embedding adapter path: {forbidden}"
                );
            }
        }
    }

    #[test]
    fn horizon_report_requires_monotonic_multi_horizon_errors() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1, 3, 5],
            ..JepaTrainingConfig::default()
        };

        let (_, outcome) = train_jepa_candidate(&long_rows(), &config).unwrap();

        assert!(outcome.horizon.e_1.is_some());
        assert!(outcome.horizon.e_3.is_some());
        assert!(outcome.horizon.e_5.is_some());
    }

    #[test]
    fn nan_guard_fails_closed() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.predictor.bias[0] = f32::NAN;

        let error = model.validate_finite().unwrap_err();

        assert!(error.to_string().contains("non-finite"));
    }

    #[test]
    fn training_run_ledger_records_component_losses() {
        let temp = tempfile::tempdir().unwrap();
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (_, outcome) = train_jepa_candidate(&rows(), &config).unwrap();

        let path = append_jepa_training_run(temp.path(), &outcome).unwrap();
        let content = std::fs::read_to_string(path).unwrap();

        assert!(content.contains("\"loss_jepa\""));
        assert!(content.contains("\"loss_var\""));
        assert!(content.contains("\"collapse\""));
        assert!(content.contains("\"backend_execution\""));
    }

    #[test]
    fn jepa_safetensors_checkpoint_roundtrips_weights() {
        let temp = tempfile::tempdir().unwrap();
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (model, _) = train_jepa_candidate(&rows(), &config).unwrap();

        let record = write_jepa_safetensors_checkpoint(temp.path(), &model).unwrap();
        let loaded = read_jepa_safetensors_checkpoint(&record.path).unwrap();

        assert_eq!(record.format, "candle_safetensors");
        assert_eq!(
            loaded.predictor_bias, model.predictor.bias,
            "predictor bias should roundtrip through the checkpoint"
        );
    }

    #[test]
    fn jepa_mlx_array_checkpoint_roundtrips_weights() {
        let temp = tempfile::tempdir().unwrap();
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let (mut model, _) = train_jepa_candidate(&rows(), &config).unwrap();
        model.metadata.backend = BackendKind::Metal;

        let record = write_jepa_mlx_array_checkpoint(temp.path(), &model).unwrap();
        let loaded = read_jepa_mlx_array_checkpoint(&record.path).unwrap();

        assert_eq!(record.format, "mlx_array");
        assert!(
            record
                .path
                .ends_with(format!("{}.mlx", model.metadata.model_id))
        );
        assert_eq!(loaded.dtype, "f32");
        assert_eq!(loaded.memory_order, "row_major");
        assert_eq!(loaded.arrays.predictor_bias, model.predictor.bias);
    }
