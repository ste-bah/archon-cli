//! Non-executing shadow observer around [`ExecutiveLoop`].
//!
//! The executive loop plans, gates and verifies; the live agent executes. This
//! runs the loop on real turns so its plans become measurable, while keeping it
//! strictly downstream of everything the user sees:
//!
//! * the executor is [`NoopActionExecutor`] — no action is taken, ever;
//! * reflection is disabled for the shadow run, because a no-op executor
//!   "succeeding" is not evidence and a lesson drawn from it would be a
//!   fabrication (issue #76's explicit warning);
//! * the observation is persisted before the live turn runs and joined to the
//!   real outcome after finalisation, so the label is the live turn's, not the
//!   shadow's;
//! * every entry point returns `Ok(None)` rather than an error when the loop
//!   has nothing to say, so a caller can wire it on the turn path without
//!   inventing failure handling.

use std::path::{Path, PathBuf};

use archon_policy::CognitivePolicy;
use cozo::DbInstance;
use uuid::Uuid;

use crate::metrics::emit::MetricEmitter;
use crate::metrics::emit::runtime_cohort;
use crate::metrics::event::MetricEventKind;
use crate::shadow::store;
use crate::shadow::types::{
    LiveTurnOutcome, ShadowComparison, ShadowObservation, now, surprise_of,
};
use crate::{
    CandidateActionKind, CognitiveConfig, CognitiveError, CognitiveSurface, DecisionRecord,
    ExecutiveLoop, ExecutiveTurnInput, NoopActionExecutor, NoopLessonSink, WorldModelScorer,
    WorldModelState,
};

/// Metric name for the shadow-vs-live comparison. Shared with the definition
/// table so a rename cannot leave events that no definition derives.
pub const SHADOW_AGREEMENT_METRIC: &str = "shadow_action_agreement_rate";

/// Marker attached to every shadow snapshot.
///
/// Present so a reader of a stored shadow row can never mistake the no-op
/// executor's report for evidence that something ran.
pub const SHADOW_DEGRADED_MARKER: &str = "shadow_only:no_action_executed";

#[derive(Debug, Clone)]
pub struct ShadowTurnInput {
    pub user_text: String,
    pub session_id: String,
    pub turn_number: u64,
    pub surface: CognitiveSurface,
    pub working_dir: PathBuf,
    pub world_model_state: WorldModelState,
    /// Model the live turn is running on, recorded as cohort identity so the
    /// agreement rate is never reported as one aggregate across models.
    pub model_id: String,
}

pub struct ShadowTurnObserver<'a> {
    db: &'a DbInstance,
    ledger_dir: PathBuf,
    config: CognitiveConfig,
    policy: Option<CognitivePolicy>,
}

impl<'a> ShadowTurnObserver<'a> {
    pub fn new(
        db: &'a DbInstance,
        ledger_dir: impl AsRef<Path>,
        config: CognitiveConfig,
        policy: Option<CognitivePolicy>,
    ) -> Self {
        Self {
            db,
            ledger_dir: ledger_dir.as_ref().to_path_buf(),
            config,
            policy,
        }
    }

    /// Run the executive loop over a turn that is about to happen.
    ///
    /// `Ok(None)` when the loop declined to plan: disabled, a trivial turn, or
    /// every candidate denied by policy. Those are not failures and must not be
    /// recorded as observations.
    pub fn observe(
        &self,
        input: ShadowTurnInput,
    ) -> Result<Option<ShadowObservation>, CognitiveError> {
        if !self.config.enabled {
            return Ok(None);
        }
        let loop_ = ExecutiveLoop::with_components(
            self.db,
            self.shadow_config(),
            self.policy.clone(),
            &self.ledger_dir,
            WorldModelScorer::heuristic_only(),
            NoopActionExecutor,
            NoopLessonSink,
        )?;
        let outcome = loop_.run_turn(ExecutiveTurnInput {
            user_text: input.user_text,
            session_id: input.session_id.clone(),
            turn_number: input.turn_number,
            surface: input.surface,
            working_dir: input.working_dir,
            world_model_state: input.world_model_state,
            // The live turn already classified and stored this situation.
            record_situation: false,
        })?;
        let Some(decision) = outcome.decision else {
            return Ok(None);
        };
        let mut degraded = outcome.snapshot.degraded.clone();
        degraded.push(SHADOW_DEGRADED_MARKER.to_string());
        let observation = ShadowObservation {
            shadow_decision_id: Uuid::new_v4().to_string(),
            session_id: input.session_id,
            turn_number: input.turn_number,
            candidate_rank: selected_rank(&decision),
            candidate_id: decision.selected_candidate_id.clone(),
            decision_id: decision.decision_id.clone(),
            situation_id: decision.situation_id.clone(),
            situation_kind: outcome.snapshot.situation_kind,
            selected_action: outcome.snapshot.selected_action,
            degraded,
            created_at: now(),
        };
        store::put_pending(self.db, &observation)?;
        // The full record goes to its own ledger so nothing is lost, while the
        // relation and the live ledger stay free of plans nobody executed.
        append_shadow_ledger(&self.ledger_dir, &observation, &decision)?;
        Ok(Some(observation))
    }

    /// Attach the real outcome to a shadow plan and emit the comparison metric.
    ///
    /// `Ok(None)` when the turn had no shadow plan to join.
    pub fn join(
        &self,
        session_id: &str,
        turn_number: u64,
        live: &LiveTurnOutcome,
        model_id: &str,
    ) -> Result<Option<ShadowComparison>, CognitiveError> {
        // The join can run in a process that never observed anything (a
        // restart between turn start and finish), so the relations may not
        // exist yet.
        crate::ensure_cognitive_schema(self.db)?;
        let Some(observation) = store::take_pending(self.db, session_id, turn_number)? else {
            return Ok(None);
        };
        let surprise = surprise_of(observation.selected_action, live);
        // With no observed action class there is nothing to agree or disagree
        // with. Recording `false` would manufacture a disagreement and bias the
        // rate downward, so the row is joined with both columns null.
        let agreed = live
            .observed_action
            .map(|observed| observation.selected_action == Some(observed));
        store::mark_joined(self.db, &observation, live, agreed, surprise)?;

        let metric_recorded = match (agreed, surprise) {
            (Some(agreed), Some(surprise)) => {
                self.record_comparison(&observation, live, agreed, surprise, model_id)?
            }
            _ => false,
        };
        Ok(Some(ShadowComparison {
            shadow_decision_id: observation.shadow_decision_id,
            decision_id: observation.decision_id,
            situation_kind: observation.situation_kind,
            shadow_action: observation.selected_action,
            live_action: live.observed_action,
            agreed,
            surprise,
            metric_recorded,
        }))
    }

    /// Config the shadow run uses, which is deliberately not the live config.
    ///
    /// * Reflection is forced off: `run_turn` reflects on whatever the executor
    ///   reported, and the shadow executor reports a success it never earned.
    ///   Persisting that would put a fabricated lesson into the same relation
    ///   the governed-proposal path reads from.
    /// * Decision recording is forced off so `cognitive_decisions` keeps
    ///   meaning "decisions the live agent was advised on". Shadow plans land in
    ///   `cognitive_shadow_decisions` and their own ledger instead, where no
    ///   count can mistake them for live ones.
    fn shadow_config(&self) -> CognitiveConfig {
        CognitiveConfig {
            record_reflections: false,
            record_decisions: false,
            ..self.config.clone()
        }
    }

    fn record_comparison(
        &self,
        observation: &ShadowObservation,
        live: &LiveTurnOutcome,
        agreed: bool,
        surprise: f32,
        model_id: &str,
    ) -> Result<bool, CognitiveError> {
        let emitter = MetricEmitter::open(
            self.db,
            &self.ledger_dir,
            runtime_cohort(
                observation.situation_kind.as_str(),
                model_id,
                self.policy.as_ref(),
            ),
        )?;
        let mut event = emitter
            .event(
                SHADOW_AGREEMENT_METRIC,
                MetricEventKind::ShadowDecisionCompared,
                &observation.shadow_decision_id,
                now(),
            )
            .with_session(&observation.session_id, observation.turn_number)
            .with_value(f64::from(surprise))
            .with_outcome(live.outcome_status())
            .with_identity("shadow_decision_id", &observation.shadow_decision_id)
            .with_identity("decision_id", &observation.decision_id)
            .with_identity("live_action_id", &live.live_action_id)
            .with_identity("candidate_id", &observation.candidate_id)
            .with_identity("candidate_rank", observation.candidate_rank.to_string())
            .with_identity("agreed", bool_identity(agreed))
            .with_identity("user_corrected", bool_identity(live.user_corrected))
            .with_identity(
                "shadow_action",
                observation
                    .selected_action
                    .map(CandidateActionKind::as_str)
                    .unwrap_or("none"),
            )
            .with_identity(
                "live_action",
                live.observed_action
                    .map(CandidateActionKind::as_str)
                    .unwrap_or("none"),
            );
        // The label comes from the live turn, never from the shadow executor.
        event.label_source = "live_turn_observation".into();
        event.evidence_refs = vec![
            format!("shadow_decision:{}", observation.shadow_decision_id),
            format!("cognitive_decision:{}", observation.decision_id),
            format!("live_action:{}", live.live_action_id),
        ];
        Ok(matches!(
            emitter.record(&event)?,
            crate::metrics::MetricWriteOutcome::Written
        ))
    }
}

/// Ledger file for shadow plans, kept apart from `cognitive-decisions.jsonl`.
const SHADOW_LEDGER: &str = "cognitive-shadow-decisions.jsonl";

fn append_shadow_ledger(
    dir: &Path,
    observation: &ShadowObservation,
    decision: &DecisionRecord,
) -> Result<(), CognitiveError> {
    use std::io::Write;

    std::fs::create_dir_all(dir)?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(SHADOW_LEDGER))?;
    let line = serde_json::json!({
        "shadow_decision_id": observation.shadow_decision_id,
        "executed": false,
        "decision": decision,
    });
    writeln!(file, "{}", serde_json::to_string(&line)?)?;
    Ok(())
}

fn bool_identity(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

/// 1-based rank of the selected candidate among the scored candidates.
///
/// `build_decision` puts the selection first, so rank 1 is the ordinary case;
/// anything else means the policy gate or the situation override moved it, and
/// that is exactly what a shadow cohort needs to be segmentable on.
fn selected_rank(decision: &DecisionRecord) -> u64 {
    decision
        .heuristic_scores
        .iter()
        .position(|score| score.candidate_id == decision.selected_candidate_id)
        .map(|index| index as u64 + 1)
        .unwrap_or(0)
}
