#[derive(Debug, Clone)]
pub struct ExecutiveTurnInput {
    pub user_text: String,
    pub session_id: String,
    pub turn_number: u64,
    pub surface: crate::CognitiveSurface,
    pub working_dir: PathBuf,
    pub world_model_state: WorldModelState,
    /// Whether the loop should persist the situation it classified.
    ///
    /// `false` for a shadow run alongside a live turn that already stored its
    /// own classification of the same text: two rows per turn would double
    /// every situation count an operator reads, while adding nothing — the two
    /// classifications come from the same deterministic classifier.
    pub record_situation: bool,
}

/// Input to [`ExecutiveLoop::run_advisory`], the only advisory path.
///
/// A second, store-less implementation of the same plan-score-gate-select
/// pipeline used to live here as a pair of free functions, because the live
/// advisory ran before the agent held a cognitive store. The two had already
/// drifted — the free one skipped the self-model context and reported
/// `prediction_unavailable` even when a prediction was used — so it is gone and
/// the live caller opens the store it now anyway holds.
#[derive(Debug, Clone)]
pub struct ExecutiveAdvisoryInput {
    pub situation: Situation,
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

