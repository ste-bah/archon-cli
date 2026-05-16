    #[derive(Clone)]
    struct ObservedFakeJepaBackend {
        selected: BackendKind,
        device_name: Option<String>,
        transition_execution: JepaStageExecution,
        runtime_execution: JepaStageExecution,
    }

    impl ObservedFakeJepaBackend {
        fn cuda(
            device_name: Option<String>,
            transition_execution: JepaStageExecution,
            runtime_execution: JepaStageExecution,
        ) -> Self {
            Self {
                selected: BackendKind::Cuda,
                device_name,
                transition_execution,
                runtime_execution,
            }
        }
    }

    impl JepaTensorBackend for ObservedFakeJepaBackend {
        fn status(&self) -> BackendStatus {
            BackendStatus {
                requested: self.selected,
                selected: self.selected,
                framework: "fake-accelerator".into(),
                device_name: self.device_name.clone(),
                experimental: false,
                fallback_reason: None,
            }
        }

        fn probe_jepa(&self) -> JepaBackendProbeReport {
            JepaBackendProbeReport {
                status: self.status(),
                feature_compiled: jepa_backend_feature_compiled(self.selected),
                tensor_self_test_passed: true,
                native_runtime_prediction: self.runtime_execution.native,
                unavailable_reason: None,
            }
        }

        fn observed_device_name(&self) -> Option<String> {
            self.device_name.clone()
        }

        fn encode_batch(
            &self,
            encoders: &JepaEncoderSet,
            batch: &JepaFeatureBatch,
        ) -> Result<JepaStageResult<JepaEncodedBatch>> {
            CpuJepaBackend.encode_batch(encoders, batch)
        }

        fn fit_predictor(
            &self,
            latent_dim: usize,
            encoded: &JepaEncodedBatch,
        ) -> Result<JepaStageResult<JepaPredictor>> {
            CpuJepaBackend.fit_predictor(latent_dim, encoded)
        }

        fn fit_auxiliary_heads(
            &self,
            latent_dim: usize,
            encoded: &JepaEncodedBatch,
        ) -> Result<JepaStageResult<Vec<JepaAuxiliaryHead>>> {
            CpuJepaBackend.fit_auxiliary_heads(latent_dim, encoded)
        }

        fn fit_transition(
            &self,
            latent_dim: usize,
            encoded: &JepaEncodedBatch,
        ) -> Result<JepaStageResult<CpuLatentTransitionModel>> {
            let transition =
                CpuLatentTransitionModel::fit(latent_dim, &encoded_transition_examples(encoded))?;
            Ok(JepaStageResult::new(transition, self.transition_execution))
        }

        fn training_losses(
            &self,
            model: &JepaTraceModel,
            encoded: &JepaEncodedBatch,
            config: &JepaTrainingConfig,
        ) -> Result<JepaStageResult<JepaTrainingLosses>> {
            CpuJepaBackend.training_losses(model, encoded, config)
        }

        fn collapse_report(
            &self,
            encoded: &JepaEncodedBatch,
            config: &JepaTrainingConfig,
        ) -> Result<JepaCollapseReport> {
            CpuJepaBackend.collapse_report(encoded, config)
        }

        fn predict_runtime(
            &self,
            model: &JepaTraceModel,
            window: &TraceWindow,
            action: &TraceAction,
        ) -> Result<JepaStageResult<JepaRuntimePrediction>> {
            let cpu_stage = CpuJepaBackend.predict_runtime(model, window, action)?;
            let mut prediction = cpu_stage.value;
            prediction.backend = self.selected;
            prediction.execution_report = JepaRuntimeBackendReport::new(
                self.selected,
                "fake-accelerator",
                self.device_name.clone(),
                self.runtime_execution.native,
                prediction.latency_ms,
            );
            prediction.execution_report.host_fallback_count =
                self.runtime_execution.host_fallback_count;
            Ok(JepaStageResult::new(prediction, self.runtime_execution))
        }
    }

    fn fake_observed_accelerator_report(
        backend: &ObservedFakeJepaBackend,
    ) -> JepaBackendExecutionReport {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let examples = build_jepa_training_examples(&rows(), &config).unwrap();
        let feature_batch = JepaFeatureBatch::from_examples(&examples, config.latent_dim).unwrap();
        let context_encoder = JepaTraceEncoder::new("context", config.latent_dim);
        let action_encoder = JepaTraceEncoder::new("action", config.latent_dim);
        let target_encoder = JepaTraceEncoder::ema_target_from(&context_encoder, config.ema_decay);
        let encoders = JepaEncoderSet {
            context_encoder: context_encoder.clone(),
            action_encoder: action_encoder.clone(),
            target_encoder: target_encoder.clone(),
        };
        let encoded_stage = backend.encode_batch(&encoders, &feature_batch).unwrap();
        let encoded = encoded_stage.value;
        let predictor_stage = backend.fit_predictor(config.latent_dim, &encoded).unwrap();
        let auxiliary_stage = backend
            .fit_auxiliary_heads(config.latent_dim, &encoded)
            .unwrap();
        let transition_stage = backend.fit_transition(config.latent_dim, &encoded).unwrap();
        let mut metadata =
            JepaTraceModelMetadata::candidate(&config, rows().len() as u64, examples.len() as u64);
        metadata.backend = backend.selected;
        let model = JepaTraceModel {
            metadata,
            context_encoder,
            action_encoder,
            target_encoder,
            predictor: predictor_stage.value,
            auxiliary_heads: auxiliary_stage.value,
            transition_model: Some(transition_stage.value),
        };
        let loss_stage = backend.training_losses(&model, &encoded, &config).unwrap();
        let runtime_stage = backend
            .predict_runtime(&model, &examples[0].context, &examples[0].action)
            .unwrap();
        let probe = backend.probe_jepa();

        JepaBackendExecutionReport::native(
            &backend.status(),
            examples.len(),
            JepaBackendExecutionEvidence {
                device_name: backend.observed_device_name(),
                tensor_self_test_passed: probe.tensor_self_test_passed,
                encode: encoded_stage.execution,
                predictor_fit: predictor_stage.execution,
                auxiliary_fit: auxiliary_stage.execution,
                transition_fit: transition_stage.execution,
                loss_eval: loss_stage.execution,
                runtime_prediction: Some(runtime_stage.execution),
            },
        )
    }

    fn metadata_with_backend_report(
        backend: BackendKind,
        report: JepaBackendExecutionReport,
    ) -> JepaTraceModelMetadata {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };
        let mut metadata = JepaTraceModelMetadata::candidate(&config, 4, 1);
        metadata.backend = backend;
        metadata.backend_execution = report;
        metadata
    }

    #[test]
    fn observed_stage_fallback_is_reflected_in_accelerator_proof() {
        let backend = ObservedFakeJepaBackend::cuda(
            Some("fake-cuda:0".into()),
            JepaStageExecution::host_fallback(1),
            JepaStageExecution::native(),
        );

        let report = fake_observed_accelerator_report(&backend);

        assert!(!report.native_transition_fit);
        assert!(report.host_fallback_count >= 1);
        assert!(!report.native_stage_proof_passes());
        let metadata = metadata_with_backend_report(BackendKind::Cuda, report);
        let error = validate_jepa_backend_execution(&metadata).unwrap_err();
        assert!(error.to_string().contains("JepaBackendNativeStageFailed"));
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn observed_all_native_cuda_stage_proof_passes_without_cuda_hardware() {
        let backend = ObservedFakeJepaBackend::cuda(
            Some("fake-cuda:0".into()),
            JepaStageExecution::native(),
            JepaStageExecution::native(),
        );

        let report = fake_observed_accelerator_report(&backend);

        assert!(report.native_encode);
        assert!(report.native_predictor_fit);
        assert!(report.native_auxiliary_fit);
        assert!(report.native_transition_fit);
        assert!(report.native_loss_eval);
        assert_eq!(report.native_runtime_prediction, Some(true));
        assert_eq!(report.host_fallback_count, 0);
        assert!(report.native_stage_proof_passes());
        let metadata = metadata_with_backend_report(BackendKind::Cuda, report);
        validate_jepa_backend_execution(&metadata).unwrap();
    }

    #[cfg(feature = "cuda")]
    #[test]
    fn accelerator_proof_requires_observed_device_name() {
        let backend = ObservedFakeJepaBackend::cuda(
            None,
            JepaStageExecution::native(),
            JepaStageExecution::native(),
        );

        let report = fake_observed_accelerator_report(&backend);

        assert_eq!(report.device_name, None);
        assert!(!report.native_stage_proof_passes());
        let metadata = metadata_with_backend_report(BackendKind::Cuda, report);
        let error = validate_jepa_backend_execution(&metadata).unwrap_err();
        assert!(error.to_string().contains("JepaBackendNativeStageFailed"));
    }

