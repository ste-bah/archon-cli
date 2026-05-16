    #[test]
    fn jepa_examples_follow_configured_horizons() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1, 3],
            ..JepaTrainingConfig::default()
        };

        let examples = build_jepa_training_examples(&rows(), &config).unwrap();

        assert!(examples.iter().any(|example| example.horizon == 1));
        assert!(examples.iter().any(|example| example.horizon == 3));
    }

    #[test]
    fn masking_uses_typed_sentinels_without_touching_target() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            mask_ratio: 1.0,
            ..JepaTrainingConfig::default()
        };
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();

        let (masked, report) = mask_jepa_training_examples(&examples, config.mask_ratio);

        assert!(report.masked_context_fields > 0);
        assert!(report.masked_action_fields > 0);
        assert_eq!(masked[0].context.session_id, examples[0].context.session_id);
        assert_eq!(
            masked[0].context.rows[0].redacted_excerpt.as_deref(),
            Some("[MASKED_EXCERPT]")
        );
        assert_eq!(masked[0].action.summary, "[MASKED_EXCERPT]");
        assert_eq!(
            masked[0].target.rows[0].redacted_excerpt,
            examples[0].target.rows[0].redacted_excerpt
        );
        assert!(!report.reconstructs_raw_text);
    }

    #[test]
    fn jepa_training_produces_configured_latent_dimensions() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };

        let (model, outcome) = train_jepa_candidate(&rows(), &config).unwrap();
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();
        let state = model.encode_state(&examples[0].context).unwrap();
        let action = model.encode_action(&examples[0].action).unwrap();
        let target = model.encode_target(&examples[0].target).unwrap();

        assert_eq!(model.metadata.model_kind, JEPA_MODEL_KIND);
        assert_eq!(model.dimensions(), 8);
        assert_eq!(state.len(), 8);
        assert_eq!(action.len(), 8);
        assert_eq!(target.len(), 8);
        assert!(outcome.losses.loss_total.is_finite());
        assert!(outcome.metadata.target_stop_gradient);
        assert_eq!(outcome.masking.mask_ratio, 0.30);
        assert!(model.transition_model.is_some());
        assert_eq!(model.provider_name(), "archon-jepa");
    }

    #[test]
    fn jepa_cpu_training_records_backend_execution_proof() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };

        let (model, outcome) =
            train_jepa_candidate_with_backend(&rows(), &config, BackendKind::Cpu, true).unwrap();

        assert_eq!(model.metadata.backend, BackendKind::Cpu);
        assert_eq!(
            outcome.metadata.backend_execution.requested_backend,
            BackendKind::Cpu
        );
        assert_eq!(
            outcome.metadata.backend_execution.selected_backend,
            BackendKind::Cpu
        );
        assert_eq!(
            model.metadata.backend_execution,
            outcome.metadata.backend_execution
        );
        assert!(outcome.metadata.backend_execution.feature_compiled);
        assert!(outcome.metadata.backend_execution.tensor_self_test_passed);
        assert!(outcome.metadata.backend_execution.native_encode);
        assert!(outcome.metadata.backend_execution.native_predictor_fit);
        assert!(outcome.metadata.backend_execution.native_auxiliary_fit);
        assert!(outcome.metadata.backend_execution.native_transition_fit);
        assert!(outcome.metadata.backend_execution.native_loss_eval);
        assert_eq!(outcome.metadata.backend_execution.host_fallback_count, 0);
    }

