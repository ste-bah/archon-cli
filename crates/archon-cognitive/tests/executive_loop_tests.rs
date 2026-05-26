use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use archon_cognitive::{
    ActionExecution, ActionExecutor, ActionOutcome, Candidate, CandidateActionKind,
    CognitiveConfig, CognitiveError, CognitiveSurface, ExecutiveLoop, ExecutiveTurnInput,
    ModelPrediction, OutcomeSummary, PlannedActionInput, PredictionBackend, PredictionDimensions,
    RiskLevel, ScoreSource, Situation, SituationClassifier, VerificationVerdict, WorldModelScorer,
    WorldModelState,
};
use archon_policy::CognitivePolicy;
use chrono::Utc;
use cozo::DbInstance;

#[derive(Clone)]
struct RecordingExecutor {
    contracts: Arc<Mutex<Vec<bool>>>,
    outcome: ActionOutcome,
}

impl ActionExecutor for RecordingExecutor {
    fn execute(&self, input: ActionExecution<'_>) -> Result<ActionOutcome, CognitiveError> {
        self.contracts
            .lock()
            .unwrap()
            .push(input.contract.is_some());
        Ok(self.outcome.clone())
    }
}

#[derive(Clone)]
struct UsefulBackend;

impl PredictionBackend for UsefulBackend {
    fn predict(
        &self,
        candidate: &Candidate,
        _state: &WorldModelState,
    ) -> Result<ModelPrediction, CognitiveError> {
        Ok(ModelPrediction {
            prediction_id: format!("prediction-{}", candidate.id),
            dimensions: PredictionDimensions {
                predicted_risk: 0.1,
                retry_pressure: 0.1,
                verification_need: 0.1,
                plan_drift_probability: 0.1,
                likely_tool_failure: 0.1,
                similarity_to_prior_failures: 0.0,
                expected_usefulness: 0.9,
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

fn executor(outcome: OutcomeSummary) -> RecordingExecutor {
    RecordingExecutor {
        contracts: Arc::new(Mutex::new(Vec::new())),
        outcome: ActionOutcome {
            outcome,
            evidence: Vec::new(),
            message: "executed".into(),
        },
    }
}

fn turn(text: &str, dir: PathBuf) -> ExecutiveTurnInput {
    ExecutiveTurnInput {
        user_text: text.into(),
        session_id: "session-1".into(),
        turn_number: 1,
        surface: CognitiveSurface::Cli,
        working_dir: dir,
        world_model_state: WorldModelState::default(),
    }
}

fn classify(text: &str) -> Situation {
    SituationClassifier.classify(archon_cognitive::ClassifyInput {
        user_text: text,
        session_id: "session-1",
        turn_number: 1,
        surface: CognitiveSurface::Cli,
    })
}

fn candidate(kind: CandidateActionKind, risk: RiskLevel) -> Candidate {
    Candidate {
        id: format!("candidate-{}", kind.as_str()),
        situation_id: "situation-1".into(),
        action_kind: kind,
        tool_name: Some("Bash".into()),
        expected_evidence: "test output".into(),
        expected_user_output: "verified result".into(),
        risk_class: risk,
        rollback_path: None,
        heuristic_score: 0.8,
        score_source: ScoreSource::Heuristic,
        created_at: Utc::now(),
    }
}

#[test]
fn hello_short_circuits_before_tools() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let exec = executor(OutcomeSummary::Success);
    let calls = exec.contracts.clone();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        temp.path(),
        WorldModelScorer::heuristic_only(),
        exec,
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();

    let result = loop_.run_turn(turn("hello", temp.path().into())).unwrap();

    assert_eq!(result.snapshot.stage, "direct_answer");
    assert!(result.decision.is_none());
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn code_edit_requires_contract_before_act() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let exec = executor(OutcomeSummary::Success);
    let calls = exec.contracts.clone();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        temp.path(),
        WorldModelScorer::heuristic_only(),
        exec,
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();
    let situation = classify("fix the failing rust test");

    loop_
        .run_planned_action(PlannedActionInput {
            situation,
            candidates: vec![candidate(CandidateActionKind::RunTests, RiskLevel::Medium)],
            working_dir: temp.path().into(),
            world_model_state: WorldModelState::default(),
            degraded: Vec::new(),
        })
        .unwrap();

    assert_eq!(*calls.lock().unwrap(), vec![true]);
}

#[test]
fn no_model_records_prediction_unavailable() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        temp.path(),
        WorldModelScorer::heuristic_only(),
        executor(OutcomeSummary::Success),
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();

    let result = loop_
        .run_turn(turn("fix the failing rust test", temp.path().into()))
        .unwrap();

    assert!(!result.snapshot.prediction_available);
    assert!(
        result
            .snapshot
            .degraded
            .contains(&"prediction_unavailable".into())
    );
}

#[test]
fn policy_missing_fails_closed_to_direct_or_clarification() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        None,
        temp.path(),
        WorldModelScorer::heuristic_only(),
        executor(OutcomeSummary::Success),
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();

    let result = loop_
        .run_turn(turn("fix the failing rust test", temp.path().into()))
        .unwrap();

    assert!(matches!(
        result.snapshot.selected_action,
        Some(CandidateActionKind::AnswerDirectly | CandidateActionKind::AskClarification)
    ));
}

#[test]
fn meaningful_failure_reflects() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        temp.path(),
        WorldModelScorer::heuristic_only(),
        executor(OutcomeSummary::Failure),
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();

    let result = loop_
        .run_planned_action(PlannedActionInput {
            situation: classify("fix the failing rust test"),
            candidates: vec![candidate(CandidateActionKind::RunTests, RiskLevel::Medium)],
            working_dir: temp.path().into(),
            world_model_state: WorldModelState::default(),
            degraded: Vec::new(),
        })
        .unwrap();

    assert!(result.snapshot.reflection_id.is_some());
    assert!(matches!(
        result.verification,
        VerificationVerdict::Skipped { .. }
    ));
}

#[test]
fn snapshot_has_no_raw_text() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        temp.path(),
        WorldModelScorer::new(UsefulBackend, true, false),
        executor(OutcomeSummary::Success),
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();
    let mut input = turn("fix hyperspecific_SECRET_word", temp.path().into());
    input.world_model_state.active_model_id = Some("model-1".into());
    input.world_model_state.active_model_kind = Some(archon_cognitive::ModelKind::LatentTransition);

    let result = loop_.run_turn(input).unwrap();
    let json = serde_json::to_string(&result.snapshot).unwrap();

    assert!(!json.contains("hyperspecific_SECRET_word"));
    assert!(result.snapshot.prediction_available);
}

#[test]
fn storage_degraded_continues() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let ledger_file = temp.path().join("ledger-as-file");
    std::fs::write(&ledger_file, "not a directory").unwrap();
    let loop_ = ExecutiveLoop::with_components(
        &db,
        config(),
        Some(policy()),
        &ledger_file,
        WorldModelScorer::heuristic_only(),
        executor(OutcomeSummary::Failure),
        archon_cognitive::NoopLessonSink,
    )
    .unwrap();

    let result = loop_
        .run_planned_action(PlannedActionInput {
            situation: classify("fix the failing rust test"),
            candidates: vec![candidate(CandidateActionKind::RunTests, RiskLevel::Medium)],
            working_dir: temp.path().into(),
            world_model_state: WorldModelState::default(),
            degraded: Vec::new(),
        })
        .unwrap();

    assert_eq!(result.action_message, "executed");
    assert!(
        result
            .snapshot
            .degraded
            .iter()
            .any(|item| item.contains("failed"))
    );
}
