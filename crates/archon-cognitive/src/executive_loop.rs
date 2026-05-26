use std::path::{Path, PathBuf};

use archon_policy::CognitivePolicy;
use cozo::DbInstance;

use crate::executive_support::*;
use crate::self_model::{MemoryContext, SelfModelProfile, SelfModelStore};
use crate::world_model_scoring::NoopPredictionBackend;
use crate::{
    Candidate, CandidatePlanner, ClassifyInput, CognitiveConfig, CognitiveError, DecisionRecord,
    ExecutiveStateSnapshot, LessonSink, NoopLessonSink, OutcomeSummary, PolicyGate, ReflectInput,
    ReflectionWriter, Situation, SituationClassifier, SituationKind, VerificationContract,
    VerificationEngine, VerificationEvidence, VerificationVerdict, WorldModelScorer,
    WorldModelState,
};

#[derive(Debug, Clone)]
pub struct ExecutiveTurnInput {
    pub user_text: String,
    pub session_id: String,
    pub turn_number: u64,
    pub surface: crate::CognitiveSurface,
    pub working_dir: PathBuf,
    pub world_model_state: WorldModelState,
}

#[derive(Debug, Clone)]
pub struct PlannedActionInput {
    pub situation: Situation,
    pub candidates: Vec<Candidate>,
    pub working_dir: PathBuf,
    pub world_model_state: WorldModelState,
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOutcome {
    pub outcome: OutcomeSummary,
    pub evidence: Vec<VerificationEvidence>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutiveRunOutcome {
    pub snapshot: ExecutiveStateSnapshot,
    pub decision: Option<DecisionRecord>,
    pub action_message: String,
    pub verification: VerificationVerdict,
}

pub struct ActionExecution<'a> {
    pub situation: &'a Situation,
    pub candidate: &'a Candidate,
    pub contract: Option<&'a VerificationContract>,
}

pub trait ActionExecutor {
    fn execute(&self, input: ActionExecution<'_>) -> Result<ActionOutcome, CognitiveError>;
}

#[derive(Debug, Clone, Default)]
pub struct NoopActionExecutor;

impl ActionExecutor for NoopActionExecutor {
    fn execute(&self, input: ActionExecution<'_>) -> Result<ActionOutcome, CognitiveError> {
        Ok(ActionOutcome {
            outcome: OutcomeSummary::Success,
            evidence: Vec::new(),
            message: format!("selected {}", input.candidate.action_kind.as_str()),
        })
    }
}

pub struct ExecutiveLoop<'a, B = NoopPredictionBackend, E = NoopActionExecutor, S = NoopLessonSink>
{
    pub(crate) db: &'a DbInstance,
    pub(crate) config: CognitiveConfig,
    policy_gate: PolicyGate,
    scorer: WorldModelScorer<B>,
    executor: E,
    lesson_sink: S,
    pub(crate) ledger_dir: PathBuf,
    classifier: SituationClassifier,
    verifier: VerificationEngine,
}

impl<'a> ExecutiveLoop<'a> {
    pub fn new(
        db: &'a DbInstance,
        config: CognitiveConfig,
        policy: Option<CognitivePolicy>,
        ledger_dir: impl AsRef<Path>,
    ) -> Result<Self, CognitiveError> {
        Self::with_components(
            db,
            config,
            policy,
            ledger_dir,
            WorldModelScorer::heuristic_only(),
            NoopActionExecutor,
            NoopLessonSink,
        )
    }
}

impl<'a, B, E, S> ExecutiveLoop<'a, B, E, S>
where
    B: crate::PredictionBackend,
    E: ActionExecutor,
    S: LessonSink,
{
    pub fn with_components(
        db: &'a DbInstance,
        config: CognitiveConfig,
        policy: Option<CognitivePolicy>,
        ledger_dir: impl AsRef<Path>,
        scorer: WorldModelScorer<B>,
        executor: E,
        lesson_sink: S,
    ) -> Result<Self, CognitiveError> {
        crate::ensure_cognitive_schema(db)?;
        Ok(Self {
            db,
            config,
            policy_gate: PolicyGate::new(policy),
            scorer,
            executor,
            lesson_sink,
            ledger_dir: ledger_dir.as_ref().to_path_buf(),
            classifier: SituationClassifier,
            verifier: VerificationEngine,
        })
    }

    pub fn run_turn(
        &self,
        input: ExecutiveTurnInput,
    ) -> Result<ExecutiveRunOutcome, CognitiveError> {
        if !self.config.enabled {
            return Ok(disabled_outcome(input));
        }
        let situation = self.classifier.classify(ClassifyInput {
            user_text: &input.user_text,
            session_id: &input.session_id,
            turn_number: input.turn_number,
            surface: input.surface,
        });
        let mut degraded = Vec::new();
        store_situation(self.db, &situation, &mut degraded);
        if situation.kind.is_trivial() {
            return Ok(direct_outcome(&situation, "direct_answer", degraded));
        }
        let (profile, memory) = self.load_context(situation.kind, &mut degraded);
        let candidates = self.plan_candidates(&situation, &profile, &memory, &mut degraded)?;
        self.run_planned_action(PlannedActionInput {
            situation,
            candidates,
            working_dir: input.working_dir,
            world_model_state: input.world_model_state,
            degraded,
        })
    }

    pub fn run_planned_action(
        &self,
        input: PlannedActionInput,
    ) -> Result<ExecutiveRunOutcome, CognitiveError> {
        let scored = self
            .scorer
            .score(&input.candidates, &input.world_model_state);
        let (allowed, denied) = self.policy_gate.filter(scored.candidates.clone());
        let Some(selected) = select_candidate(allowed.clone(), &input.situation) else {
            return Ok(direct_outcome(
                &input.situation,
                "policy_blocked",
                input.degraded,
            ));
        };

        let mut degraded = input.degraded;
        if !scored.prediction_available {
            degraded.push("prediction_unavailable".into());
        }
        let contract = self.contract_for(&input.situation, &selected, &input.working_dir)?;
        let mut decision = build_decision(
            &input.situation,
            &selected,
            &allowed,
            &scored.candidates,
            &denied,
            self.policy_gate.verdict(&denied),
        )?;
        decision.verification_contract = contract_json(&contract)?;
        record_decision(self, &decision, &mut degraded);

        let action = self.execute_action(
            &input.situation,
            &selected,
            contract.as_ref(),
            &mut degraded,
        );
        let verification = verify_action(&self.verifier, contract.as_ref(), &action);
        let reflection_id = self.reflect(
            &input.situation,
            &decision,
            &verification,
            &action,
            &mut degraded,
        );
        Ok(ExecutiveRunOutcome {
            snapshot: snapshot(SnapshotParams {
                situation: &input.situation,
                stage: "complete",
                selected: Some(&selected),
                policy_summary: decision.policy_verdict.clone().unwrap_or_default(),
                verification_summary: verification_summary(&verification),
                prediction_available: scored.prediction_available,
                reflection_id,
                degraded,
            }),
            decision: Some(decision),
            action_message: action.message,
            verification,
        })
    }

    fn load_context(
        &self,
        kind: SituationKind,
        degraded: &mut Vec<String>,
    ) -> (SelfModelProfile, MemoryContext) {
        if !self.config.use_self_model {
            return (neutral_profile(kind), MemoryContext::default());
        }
        let domain = domain_for(kind);
        match SelfModelStore::new(self.db) {
            Ok(store) => load_store_context(&store, domain, degraded),
            Err(error) => {
                degraded.push(format!("self_model_unavailable:{error}"));
                (neutral_profile(kind), MemoryContext::default())
            }
        }
    }

    fn plan_candidates(
        &self,
        situation: &Situation,
        profile: &SelfModelProfile,
        memory: &MemoryContext,
        degraded: &mut Vec<String>,
    ) -> Result<Vec<Candidate>, CognitiveError> {
        let planner = CandidatePlanner::new(self.db, self.config.max_candidates)
            .unwrap_or_else(|_| CandidatePlanner::without_store(self.config.max_candidates));
        planner
            .generate(situation, profile, memory)
            .inspect_err(|error| {
                degraded.push(format!("candidate_planning_failed:{error}"));
            })
    }

    fn contract_for(
        &self,
        situation: &Situation,
        candidate: &Candidate,
        working_dir: &Path,
    ) -> Result<Option<VerificationContract>, CognitiveError> {
        let Some(kind) = verification_kind(situation.kind, candidate) else {
            return Ok(None);
        };
        self.verifier
            .require(&crate::ContractInput {
                verification_kind: kind,
                action_kind: candidate.action_kind,
                files_touched: Vec::new(),
                commands_planned: candidate.tool_name.clone().into_iter().collect(),
                working_directory: working_dir.to_path_buf(),
                situation_id: situation.id.clone(),
                override_reason: Some("executive loop governed action".into()),
            })
            .map(Some)
    }

    fn execute_action(
        &self,
        situation: &Situation,
        selected: &Candidate,
        contract: Option<&VerificationContract>,
        degraded: &mut Vec<String>,
    ) -> ActionOutcome {
        self.executor
            .execute(ActionExecution {
                situation,
                candidate: selected,
                contract,
            })
            .unwrap_or_else(|error| {
                degraded.push(format!("action_executor_failed:{error}"));
                ActionOutcome {
                    outcome: OutcomeSummary::Failure,
                    evidence: Vec::new(),
                    message: "action executor failed".into(),
                }
            })
    }

    fn reflect(
        &self,
        situation: &Situation,
        decision: &DecisionRecord,
        verification: &VerificationVerdict,
        action: &ActionOutcome,
        degraded: &mut Vec<String>,
    ) -> Option<String> {
        let writer = ReflectionWriter::with_lesson_sink(
            self.db,
            &self.ledger_dir,
            self.config.record_reflections,
            self.lesson_sink.clone(),
        )
        .ok()?;
        let outcome = reflection_outcome(action.outcome, verification);
        let result = writer.reflect(ReflectInput {
            decision: decision.clone(),
            situation_kind: situation.kind,
            verification: verification.clone(),
            outcome,
            user_corrected: false,
        });
        match result {
            Ok(outcome) => {
                degraded.extend(outcome.degraded);
                outcome.reflection.map(|record| record.reflection_id)
            }
            Err(error) => {
                degraded.push(format!("reflection_failed:{error}"));
                None
            }
        }
    }
}
