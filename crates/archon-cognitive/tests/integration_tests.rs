use archon_cognitive::{
    ActionExecution, ActionExecutor, ActionOutcome, Candidate, CandidateActionKind,
    CognitiveConfig, CognitiveError, CognitiveInspection, CognitiveSurface, ExecutiveLoop,
    ExecutiveTurnInput, ModelKind, ModelPrediction, OutcomeSummary, PredictionBackend,
    PredictionDimensions, ScoreSource, SituationKind, VerificationEvidence, VerificationVerdict,
    WorldModelScorer, WorldModelState,
};
use archon_policy::CognitivePolicy;
use cozo::DbInstance;
use tempfile::TempDir;

#[derive(Debug, Clone)]
struct PassingExecutor;

impl ActionExecutor for PassingExecutor {
    fn execute(&self, input: ActionExecution<'_>) -> Result<ActionOutcome, CognitiveError> {
        let evidence = input
            .contract
            .map(|contract| {
                contract
                    .requirements
                    .iter()
                    .map(|requirement| VerificationEvidence {
                        evidence_type: requirement.evidence_type.clone(),
                        target: requirement.target.clone(),
                        passed: Some(true),
                        details: "integration evidence".into(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(ActionOutcome {
            outcome: OutcomeSummary::Success,
            evidence,
            message: format!("selected {}", input.candidate.action_kind.as_str()),
        })
    }
}

#[derive(Debug, Clone)]
struct UsefulPredictionBackend;

impl PredictionBackend for UsefulPredictionBackend {
    fn predict(
        &self,
        candidate: &Candidate,
        _state: &WorldModelState,
    ) -> Result<ModelPrediction, CognitiveError> {
        let bonus = if candidate.action_kind == CandidateActionKind::RunLearningTick {
            0.95
        } else {
            0.60
        };
        Ok(ModelPrediction {
            prediction_id: format!("prediction-{}", candidate.id),
            dimensions: PredictionDimensions {
                predicted_risk: 0.1,
                retry_pressure: 0.1,
                verification_need: 0.1,
                plan_drift_probability: 0.1,
                likely_tool_failure: 0.1,
                similarity_to_prior_failures: 0.0,
                expected_usefulness: bonus,
                expected_user_friction: 0.1,
            },
        })
    }
}

fn config() -> CognitiveConfig {
    CognitiveConfig {
        enabled: true,
        record_decisions: true,
        record_reflections: true,
        use_self_model: false,
        max_candidates: 5,
        ..CognitiveConfig::default()
    }
}

fn policy() -> CognitivePolicy {
    CognitivePolicy {
        enabled: true,
        max_autonomous_risk: "Medium".into(),
        ..CognitivePolicy::default()
    }
}

fn turn(text: &str, temp: &TempDir) -> ExecutiveTurnInput {
    ExecutiveTurnInput {
        user_text: text.into(),
        session_id: "integration-session".into(),
        turn_number: 1,
        surface: CognitiveSurface::Tui,
        working_dir: temp.path().into(),
        world_model_state: WorldModelState::default(),
    }
}

fn db_and_dir() -> (DbInstance, TempDir) {
    (
        DbInstance::new("mem", "", "").unwrap(),
        tempfile::tempdir().unwrap(),
    )
}

#[test]
fn greeting_flow_short_circuits_without_decision_or_reflection() {
    let (db, temp) = db_and_dir();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        temp.path(),
        WorldModelScorer::heuristic_only(),
        PassingExecutor,
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();

    let outcome = loop_.run_turn(turn("hello", &temp)).unwrap();
    let status = CognitiveInspection::new(&db, temp.path())
        .unwrap()
        .status()
        .unwrap();

    assert_eq!(outcome.snapshot.situation_kind, SituationKind::Greeting);
    assert_eq!(outcome.snapshot.stage, "direct_answer");
    assert!(outcome.decision.is_none());
    assert_eq!(status.tool_decision_count, 0);
    assert_eq!(status.reflection_count, 0);
}

#[test]
fn code_change_flow_records_decision_contract_and_reflection() {
    let (db, temp) = db_and_dir();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        temp.path(),
        WorldModelScorer::heuristic_only(),
        PassingExecutor,
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();

    let outcome = loop_
        .run_turn(turn("fix the parser and run relevant tests", &temp))
        .unwrap();
    let decision = outcome.decision.as_ref().expect("decision");
    let status = CognitiveInspection::new(&db, temp.path())
        .unwrap()
        .status()
        .unwrap();

    assert_eq!(outcome.snapshot.situation_kind, SituationKind::CodeChange);
    assert!(decision.verification_contract.is_some());
    assert_eq!(outcome.verification, VerificationVerdict::Passed);
    assert_eq!(status.executive_decision_count, 1);
    assert_eq!(status.reflection_count, 1);
}

#[test]
fn ci_debug_flow_prefers_current_log_probe_over_stale_answers() {
    let (db, temp) = db_and_dir();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        temp.path(),
        WorldModelScorer::heuristic_only(),
        PassingExecutor,
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();

    let outcome = loop_
        .run_turn(turn("why did the GitHub Actions build fail?", &temp))
        .unwrap();
    let decision = outcome.decision.as_ref().expect("decision");

    assert_eq!(outcome.snapshot.situation_kind, SituationKind::CiDebug);
    assert_eq!(
        outcome.snapshot.selected_action,
        Some(CandidateActionKind::RunSafeShellProbe)
    );
    assert!(decision.user_visible_summary.contains("safe_shell_probe"));
    assert_eq!(outcome.verification, VerificationVerdict::Passed);
}

#[test]
fn world_model_flow_links_promoted_jepa_prediction_to_decision() {
    let (db, temp) = db_and_dir();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        temp.path(),
        WorldModelScorer::new(UsefulPredictionBackend, true, true),
        PassingExecutor,
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();
    let mut input = turn("run a world model learning tick", &temp);
    input.world_model_state = WorldModelState {
        active_model_id: Some("jepa-active".into()),
        active_model_kind: Some(ModelKind::JepaTransition),
        jepa_promoted: true,
        shadow_only: false,
    };

    let outcome = loop_.run_turn(input).unwrap();
    let decision = outcome.decision.as_ref().expect("decision");

    assert_eq!(
        outcome.snapshot.situation_kind,
        SituationKind::WorldModelTask
    );
    assert!(outcome.snapshot.prediction_available);
    assert!(
        decision
            .heuristic_scores
            .iter()
            .any(|score| score.score_source == ScoreSource::JepaTransition)
    );
}
