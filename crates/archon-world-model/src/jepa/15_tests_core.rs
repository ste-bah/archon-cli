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
    fn controlled_jepa_example_build_stops_before_transition_scan() {
        let config = JepaTrainingConfig {
            latent_dim: 8,
            context_window_rows: 2,
            target_window_rows: 1,
            prediction_horizons: vec![1],
            ..JepaTrainingConfig::default()
        };

        let error =
            build_jepa_training_examples_controlled(&rows(), &config, Some(&|| true)).unwrap_err();

        assert!(error.to_string().contains("stopped or timed out"));
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

        let example = examples
            .iter()
            .find(|example| !example.context.rows.is_empty())
            .expect("fixture should include an action with prior context");
        let (masked, report) =
            mask_jepa_training_examples(std::slice::from_ref(example), config.mask_ratio);

        assert!(report.masked_context_fields > 0);
        assert!(report.masked_action_fields > 0);
        assert_eq!(masked[0].context.session_id, example.context.session_id);
        assert_eq!(
            masked[0].context.rows[0].redacted_excerpt.as_deref(),
            Some("[MASKED_EXCERPT]")
        );
        assert_eq!(masked[0].action.summary, "[MASKED_EXCERPT]");
        assert_eq!(
            masked[0].target.rows[0].redacted_excerpt,
            example.target.rows[0].redacted_excerpt
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
        assert_eq!(model.provider_name(), "archon-jepa-inspired");
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

    /// An embedding the encoder cannot use must not silently delete the
    /// excerpt text along with it.
    ///
    /// `seed_features` rejected a mismatched or non-finite vector and zeroed
    /// the seed, while the lexical fallback only checked that the embedding was
    /// non-empty and switched itself off. The two disagreed, so such a window
    /// carried neither the embedding nor the hashed text — worse than either
    /// path alone, and silent. Providers do return unexpected dimensions.
    #[test]
    fn an_unusable_embedding_keeps_the_lexical_fallback() {
        let dim = 8;
        let source = rows();
        let builder = TraceWindowBuilder::new(&source);
        let plain = builder.context_window("r2", 2).unwrap();
        let baseline = window_features(&plain, dim, "context").unwrap();

        let mut wrong_length = plain.clone();
        wrong_length.embedding = Some(vec![0.5; dim + 1]);
        assert_eq!(
            window_features(&wrong_length, dim, "context").unwrap(),
            baseline,
            "a wrong-length embedding must fall back to hashed excerpt text"
        );

        let mut not_finite = plain.clone();
        not_finite.embedding = Some(vec![f32::NAN; dim]);
        assert_eq!(
            window_features(&not_finite, dim, "context").unwrap(),
            baseline,
            "a non-finite embedding must fall back to hashed excerpt text"
        );

        let mut usable = plain.clone();
        usable.embedding = Some((0..dim).map(|idx| idx as f32 / dim as f32).collect());
        assert_ne!(
            window_features(&usable, dim, "context").unwrap(),
            baseline,
            "a usable embedding must actually change the features"
        );
    }

    fn row_categorical_weights() -> CategoricalWeights {
        CategoricalWeights {
            source: 0.45,
            action_kind: 0.65,
            provider: 0.55,
            model: 0.40,
            agent: 0.40,
        }
    }

    /// Every closed-enum value must own a dimension, with no two sharing one.
    #[test]
    fn closed_enum_categoricals_have_distinct_slots() {
        let sources = [
            WorldTraceSource::ActivityEvent,
            WorldTraceSource::PipelineBundle,
            WorldTraceSource::ProviderRuntime,
            WorldTraceSource::Plan,
            WorldTraceSource::Conversation,
            WorldTraceSource::AgentTranscript,
            WorldTraceSource::AgentOutput,
            WorldTraceSource::Workflow,
            WorldTraceSource::Retrospective,
            WorldTraceSource::Memory,
            WorldTraceSource::AgentEvolution,
            WorldTraceSource::ReasoningQuality,
        ];
        let slots: std::collections::HashSet<usize> = sources.iter().map(source_slot).collect();
        assert_eq!(slots.len(), sources.len(), "two sources share a dimension");
        assert_eq!(slots.len(), SOURCE_SLOTS, "SOURCE_SLOTS is out of step");
        assert!(slots.iter().all(|slot| *slot < SOURCE_SLOTS));

        let kinds = [
            WorldActionKind::AgentAttempt,
            WorldActionKind::ProviderCall,
            WorldActionKind::ToolCall,
            WorldActionKind::PlanUpdate,
            WorldActionKind::MemorySurface,
            WorldActionKind::Verification,
            WorldActionKind::Retry,
            WorldActionKind::Resume,
            WorldActionKind::MessageSend,
            WorldActionKind::TaskClaim,
            WorldActionKind::Handoff,
            WorldActionKind::WorktreeMerge,
            WorldActionKind::Unknown,
        ];
        let slots: std::collections::HashSet<usize> = kinds.iter().map(action_kind_slot).collect();
        assert_eq!(slots.len(), kinds.len(), "two action kinds share a dimension");
        assert_eq!(slots.len(), ACTION_KIND_SLOTS, "ACTION_KIND_SLOTS is out of step");
    }

    /// Two different sources must land on different dimensions rather than
    /// colliding in a shared hash bucket.
    #[test]
    fn reserved_slots_keep_distinct_categoricals_apart() {
        let dim = 384;
        let base = categorical_base(dim).expect("384 dimensions should reserve a block");
        assert!(base + CATEGORICAL_SLOTS <= dim, "reserved block runs off the end");

        let build = |source: &WorldTraceSource| {
            let mut features = vec![0.0_f32; dim];
            add_categorical_features(
                &mut features,
                Categoricals {
                    source: Some(source),
                    action_kind: &WorldActionKind::Retry,
                    provider: None,
                    model: None,
                    agent: None,
                },
                row_categorical_weights(),
                1.0,
                "context",
            );
            features
        };

        let memory = build(&WorldTraceSource::Memory);
        let plan = build(&WorldTraceSource::Plan);
        let memory_slot = base + SOURCE_BASE + source_slot(&WorldTraceSource::Memory);

        assert_ne!(memory[memory_slot], 0.0, "source did not reach its own slot");
        assert_eq!(
            plan[memory_slot], 0.0,
            "a different source wrote into the memory slot"
        );
    }

    /// Small latent dimensions cannot spare the block and keep hashing.
    #[test]
    fn a_small_latent_dim_keeps_the_hashed_categoricals() {
        assert!(categorical_base(8).is_none());
        assert!(categorical_base(CATEGORICAL_SLOTS * 4).is_some());
    }
