#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_promotes_and_reads_active_model() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();

        let path = registry.promote("candidate-1").unwrap();

        assert!(path.exists());
        assert_eq!(
            registry.active_model_id().unwrap().as_deref(),
            Some("candidate-1")
        );
        assert!(
            temp.path()
                .join("ledgers")
                .join("model-activations.jsonl")
                .exists()
        );
        assert_eq!(
            registry.active_model_kind().unwrap().as_deref(),
            Some(LATENT_TRANSITION_MODEL_KIND)
        );
    }

    #[test]
    fn registry_pointer_roundtrips_model_kind() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();

        registry
            .promote_model_kind("jepa-candidate-1", crate::jepa::JEPA_MODEL_KIND)
            .unwrap();

        assert_eq!(
            registry.active_model_id().unwrap().as_deref(),
            Some("jepa-candidate-1")
        );
        assert_eq!(
            registry.active_model_kind().unwrap().as_deref(),
            Some(crate::jepa::JEPA_MODEL_KIND)
        );
        let content = std::fs::read_to_string(registry.active_pointer_path()).unwrap();
        assert!(content.contains("\"model_kind\": \"jepa_transition\""));
    }

    #[test]
    fn registry_legacy_pointer_defaults_to_latent_transition_kind() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        std::fs::write(
            registry.active_pointer_path(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "model_id": "legacy-candidate",
                "previous_model_id": null,
                "updated_at": Utc::now()
            }))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            registry.active_model_kind().unwrap().as_deref(),
            Some(LATENT_TRANSITION_MODEL_KIND)
        );
    }

    #[test]
    fn registry_rollback_updates_active_pointer() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();

        registry.promote("candidate-1").unwrap();
        registry.rollback("candidate-0").unwrap();

        assert_eq!(
            registry.active_model_id().unwrap().as_deref(),
            Some("candidate-0")
        );
        let content =
            std::fs::read_to_string(temp.path().join("ledgers/model-activations.jsonl")).unwrap();
        assert!(content.contains("\"action\":\"rollback\""));
    }

    #[test]
    fn registry_writes_and_loads_cpu_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        let examples = [crate::model::LatentTransitionExample {
            state: vec![0.0, 0.0],
            action: vec![0.0, 0.0],
            next_state: vec![1.0, 1.0],
            labels: Default::default(),
        }];
        let (model, outcome) = crate::train::train_cpu_candidate(2, &examples).unwrap();

        let path = registry.write_cpu_candidate(&model, &outcome).unwrap();
        let loaded = registry
            .load_cpu_candidate(&model.metadata.model_id)
            .unwrap();

        assert!(path.exists());
        assert!(
            temp.path()
                .join("candidates")
                .join(format!("{}.safetensors", model.metadata.model_id))
                .exists()
        );
        assert_eq!(loaded.model.transition_bias, model.transition_bias);
        assert_eq!(loaded.outcome.status, outcome.status);
    }

    #[test]
    fn registry_writes_and_loads_jepa_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        let mut first =
            crate::schema::WorldTraceRow::new("s1", crate::schema::WorldActionKind::ToolCall)
                .with_row_id("r1");
        first.redacted_excerpt = Some("run tests".into());
        let mut second =
            crate::schema::WorldTraceRow::new("s1", crate::schema::WorldActionKind::Verification)
                .with_row_id("r2");
        second.redacted_excerpt = Some("tests passed".into());
        let config = crate::jepa::JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 1,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..crate::jepa::JepaTrainingConfig::default()
        };
        let (model, outcome) =
            crate::jepa::train_jepa_candidate(&[first, second], &config).unwrap();

        let path = registry.write_jepa_candidate(&model, &outcome).unwrap();
        let loaded = registry
            .load_jepa_candidate(&model.metadata.model_id)
            .unwrap();

        assert!(path.exists());
        assert!(
            temp.path()
                .join("jepa")
                .join("candidates")
                .join(format!("{}.safetensors", model.metadata.model_id))
                .exists()
        );
        assert_eq!(
            loaded.model.metadata.model_kind,
            crate::jepa::JEPA_MODEL_KIND
        );
        assert_eq!(loaded.outcome.status, outcome.status);
        assert_eq!(loaded.checkpoint.format, "candle_safetensors");
        assert!(loaded.training_run.exists());
    }

    #[test]
    fn registry_writes_metal_jepa_candidate_with_mlx_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        let mut first =
            crate::schema::WorldTraceRow::new("s1", crate::schema::WorldActionKind::ToolCall)
                .with_row_id("r1");
        first.redacted_excerpt = Some("run tests".into());
        let mut second =
            crate::schema::WorldTraceRow::new("s1", crate::schema::WorldActionKind::Verification)
                .with_row_id("r2");
        second.redacted_excerpt = Some("tests passed".into());
        let config = crate::jepa::JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 1,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..crate::jepa::JepaTrainingConfig::default()
        };
        let (mut model, mut outcome) =
            crate::jepa::train_jepa_candidate(&[first, second], &config).unwrap();
        model.metadata.backend = crate::backend::BackendKind::Metal;
        model.metadata.backend_execution = crate::jepa::JepaBackendExecutionReport {
            requested_backend: crate::backend::BackendKind::Metal,
            selected_backend: crate::backend::BackendKind::Metal,
            framework: "mlx-rs".into(),
            device_name: Some("metal:0".into()),
            commit_sha: "abc123".into(),
            feature_compiled: true,
            tensor_self_test_passed: true,
            hardware_validation_captured_at: Some(chrono::Utc::now()),
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
        if let Some(transition) = &mut model.transition_model {
            transition.metadata.backend = crate::backend::BackendKind::Metal;
        }
        outcome.metadata = model.metadata.clone();

        let path = registry.write_jepa_candidate(&model, &outcome).unwrap();
        let loaded = registry
            .load_jepa_candidate(&model.metadata.model_id)
            .unwrap();

        assert!(path.exists());
        assert!(
            temp.path()
                .join("jepa")
                .join("candidates")
                .join(format!("{}.mlx", model.metadata.model_id))
                .exists()
        );
        assert_eq!(loaded.checkpoint.format, "mlx_array");
    }

    #[test]
    fn registry_rejects_jepa_candidate_with_laundered_accelerator_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        let mut first =
            crate::schema::WorldTraceRow::new("s1", crate::schema::WorldActionKind::ToolCall)
                .with_row_id("r1");
        first.redacted_excerpt = Some("run tests".into());
        let mut second =
            crate::schema::WorldTraceRow::new("s1", crate::schema::WorldActionKind::Verification)
                .with_row_id("r2");
        second.redacted_excerpt = Some("tests passed".into());
        let config = crate::jepa::JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 1,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..crate::jepa::JepaTrainingConfig::default()
        };
        let (mut model, mut outcome) =
            crate::jepa::train_jepa_candidate(&[first, second], &config).unwrap();
        model.metadata.backend = crate::backend::BackendKind::Cuda;
        outcome.metadata = model.metadata.clone();

        let error = registry.write_jepa_candidate(&model, &outcome).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("does not match execution report")
        );
    }

    #[test]
    fn registry_writes_and_loads_eval_report() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        let record = CandidateEvalRecord {
            candidate_id: "candidate-1".into(),
            report: PromotionGateReport {
                cosine_error_improved: true,
                surprise_ks_passed: true,
                counterfactual_ndcg_passed: true,
                brier_improved: true,
                no_critical_regression: true,
            },
            next_state: None,
            surprise: None,
            brier: None,
            created_at: Utc::now(),
        };

        registry.write_eval_report(&record).unwrap();
        let loaded = registry.load_eval_report("candidate-1").unwrap().unwrap();

        assert!(loaded.report.all_primary_gates_passed());
    }

    #[test]
    fn registry_counts_candidate_records_only() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        std::fs::write(registry.candidate_record_path("c1"), "{}").unwrap();
        std::fs::write(registry.eval_record_path("c1"), "{}").unwrap();

        assert_eq!(registry.candidate_count().unwrap(), 1);
    }

    #[test]
    fn registry_loads_latest_eval_report() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ModelRegistry::open(temp.path()).unwrap();
        let record = CandidateEvalRecord {
            candidate_id: "candidate-1".into(),
            report: PromotionGateReport {
                cosine_error_improved: true,
                surprise_ks_passed: true,
                counterfactual_ndcg_passed: true,
                brier_improved: true,
                no_critical_regression: true,
            },
            next_state: None,
            surprise: None,
            brier: None,
            created_at: Utc::now(),
        };

        registry.write_eval_report(&record).unwrap();

        let loaded = registry.latest_eval_report().unwrap().unwrap();
        assert_eq!(loaded.candidate_id, "candidate-1");
    }
}
