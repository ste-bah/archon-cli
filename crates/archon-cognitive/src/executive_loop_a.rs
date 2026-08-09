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

#[derive(Debug, Clone)]
pub struct ExecutiveAdvisoryInput {
    pub situation: Situation,
    pub working_dir: PathBuf,
    pub world_model_state: WorldModelState,
}

/// Plan a runtime advisory using heuristics only.
///
/// Equivalent to [`plan_runtime_advisory_with`] passing
/// [`WorldModelScorer::heuristic_only`]; kept so callers that have no world
/// model wired do not have to name a backend.
pub fn plan_runtime_advisory(
    config: &CognitiveConfig,
    policy: CognitivePolicy,
    input: ExecutiveAdvisoryInput,
) -> Result<ExecutiveRunOutcome, CognitiveError> {
    plan_runtime_advisory_with(config, policy, input, &WorldModelScorer::heuristic_only())
}

/// Plan a runtime advisory, scoring candidates with the supplied scorer.
///
/// The scorer decides whether model predictions are consulted at all: with a
/// `shadow_only` or model-less [`WorldModelState`] it falls back to heuristic
/// scores, so passing a live backend is safe before the model has been
/// validated.
pub fn plan_runtime_advisory_with<B: PredictionBackend>(
    config: &CognitiveConfig,
    policy: CognitivePolicy,
    input: ExecutiveAdvisoryInput,
    scorer: &WorldModelScorer<B>,
) -> Result<ExecutiveRunOutcome, CognitiveError> {
    if !config.enabled || input.situation.kind.is_trivial() {
        return Ok(direct_outcome(&input.situation, "not_required", Vec::new()));
    }
    let started = std::time::Instant::now();
    let profile = neutral_profile(input.situation.kind);
    let candidates = CandidatePlanner::without_store(config.max_candidates).generate(
        &input.situation,
        &profile,
        &MemoryContext::default(),
    )?;
    let scored = scorer.score(&candidates, &input.world_model_state);
    let gate = PolicyGate::new(Some(policy));
    let (allowed, denied) = gate.filter(scored.candidates.clone());
    let Some(selected) = select_candidate(allowed.clone(), &input.situation) else {
        return Ok(direct_outcome(
            &input.situation,
            "policy_blocked",
            vec!["advisory_only:no_action_executed".into()],
        ));
    };
    let contract = advisory_contract(&input.situation, &selected, &input.working_dir)?;
    let mut decision = build_decision(
        &input.situation,
        &selected,
        &allowed,
        &scored.candidates,
        &denied,
        gate.verdict(&denied),
    )?;
    decision.verification_contract = contract_json(&contract)?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if elapsed_ms > config.max_pipeline_ms {
        return Err(CognitiveError::Store(format!(
            "runtime advisory exceeded {}ms budget",
            config.max_pipeline_ms
        )));
    }
    Ok(ExecutiveRunOutcome {
        snapshot: snapshot(SnapshotParams {
            situation: &input.situation,
            stage: "advisory",
            selected: Some(&selected),
            policy_summary: decision.policy_verdict.clone().unwrap_or_default(),
            verification_summary: "not_run".into(),
            prediction_available: scored.prediction_available,
            reflection_id: None,
            degraded: vec![
                "advisory_only:no_action_executed".into(),
                "prediction_unavailable".into(),
            ],
        }),
        decision: Some(decision),
        action_message: "advisory selection recorded; live agent retains execution authority"
            .into(),
        verification: VerificationVerdict::NotRun,
    })
}

fn advisory_contract(
    situation: &Situation,
    candidate: &Candidate,
    working_dir: &Path,
) -> Result<Option<VerificationContract>, CognitiveError> {
    let Some(kind) = verification_kind(situation.kind, candidate) else {
        return Ok(None);
    };
    VerificationEngine
        .require(&crate::ContractInput {
            verification_kind: kind,
            action_kind: candidate.action_kind,
            files_touched: Vec::new(),
            commands_planned: candidate.tool_name.clone().into_iter().collect(),
            working_directory: working_dir.to_path_buf(),
            situation_id: situation.id.clone(),
            override_reason: Some("executive loop advisory".into()),
        })
        .map(Some)
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

