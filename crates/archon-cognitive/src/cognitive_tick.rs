use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use archon_policy::CognitivePolicy;
use chrono::{DateTime, Utc};
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cognitive_tick_store::store_tick_report;
use crate::cozo_guard::{relation_count, run_script_guarded};
use crate::schema::ensure_cognitive_schema;
use crate::self_model::SelfModelWriter;
use crate::{
    CognitiveError, GovernedAutonomousApply, OutcomeSummary, ReflectionRecord, SituationKind,
};

/// Outcome of one autonomous tick.
///
/// A step that could not run reports `None` rather than a plausible zero/`true`,
/// so a reader of the audit can tell "we looked and there was nothing" apart
/// from "nobody ever looked".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickReport {
    pub tick_id: String,
    /// Ledgered reflections re-put into Cozo. `Some(0)` is a measurement
    /// ("ledger and relation agree"); `None` means the replay itself failed.
    pub dead_letters_replayed: Option<u64>,
    pub proposals_evaluated: u64,
    pub proposals_auto_applied: u64,
    pub proposals_denied: u64,
    pub proposals_generated: u64,
    /// Whether the self-model changed. `None` means policy withheld self-model
    /// updates, which is not the same claim as `Some(false)` ("it ran and
    /// nothing needed updating").
    pub self_model_updated: Option<bool>,
    pub errors: Vec<String>,
    pub duration_ms: u64,
    pub created_at: DateTime<Utc>,
}

pub struct CognitiveTick<'a> {
    db: &'a DbInstance,
    policy: Option<CognitivePolicy>,
    ledger_dir: PathBuf,
}

impl<'a> CognitiveTick<'a> {
    /// `ledger_dir` is the cognitive store root: the tick replays dead letters
    /// out of the JSONL ledgers there and emits metric events beside them, so
    /// it cannot be constructed without one.
    pub fn new(
        db: &'a DbInstance,
        policy: Option<CognitivePolicy>,
        ledger_dir: impl AsRef<Path>,
    ) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self {
            db,
            policy,
            ledger_dir: ledger_dir.as_ref().to_path_buf(),
        })
    }

    pub fn tick(&self) -> Result<TickReport, CognitiveError> {
        let started = Instant::now();
        let mut report = TickReport::empty();
        if !self
            .policy
            .as_ref()
            .is_some_and(|policy| policy.allow_autonomous_tick)
        {
            report.errors.push("tick disabled by policy".into());
            return self.finish(report, started);
        }

        report.dead_letters_replayed = self.replay_dead_letters(&mut report.errors);
        report.proposals_evaluated = self.inspect_pending_proposals(&mut report.errors);
        report.proposals_generated = self.propose_improvements(&mut report.errors);
        report.self_model_updated = self.refresh_self_model(&mut report.errors);
        self.finish(report, started)
    }

    /// Re-put reflections that reached the ledger but not the relation.
    ///
    /// `Some(0)` is now a real measurement — the two agree — and `None` is
    /// reserved for the replay itself failing, which is the only remaining case
    /// where the tick genuinely does not know.
    fn replay_dead_letters(&self, errors: &mut Vec<String>) -> Option<u64> {
        match crate::dead_letters::replay(self.db, &self.ledger_dir) {
            Ok(report) => {
                errors.extend(report.errors);
                if report.unparseable > 0 {
                    errors.push(format!(
                        "dead_letter_ledger_unparseable:{}",
                        report.unparseable
                    ));
                }
                Some(report.replayed)
            }
            Err(error) => {
                errors.push(format!("replay_dead_letters:{error}"));
                None
            }
        }
    }

    fn inspect_pending_proposals(&self, errors: &mut Vec<String>) -> u64 {
        relation_count(self.db, "governed_proposals", "proposal_id")
            .map(|count| count as u64)
            .unwrap_or_else(|error| {
                errors.push(format!("inspect_pending_proposals:{error}"));
                0
            })
    }

    fn propose_improvements(&self, errors: &mut Vec<String>) -> u64 {
        let reflections = recent_proposable_reflections(self.db, errors);
        let Ok(apply) = GovernedAutonomousApply::new(self.db, self.policy.clone()) else {
            errors.push("governed_apply_unavailable".into());
            return 0;
        };
        let mut generated = 0;
        let mut seen = BTreeSet::new();
        for reflection in reflections {
            if !seen.insert(format!(
                "{}:{}",
                reflection.situation_kind.as_str(),
                reflection.lesson
            )) {
                continue;
            }
            match apply.propose(&reflection) {
                Ok(_) => generated += 1,
                Err(error) => errors.push(format!("proposal_generation:{error}")),
            }
        }
        generated
    }

    /// Recompute domain-trust facts from verified reflection outcomes.
    ///
    /// `None` only when policy withholds self-model updates or the refresh
    /// failed; otherwise the boolean is what actually happened, and the
    /// domains that produced no fact are surfaced as errors-with-reasons rather
    /// than silently absent.
    fn refresh_self_model(&self, errors: &mut Vec<String>) -> Option<bool> {
        let writer = SelfModelWriter::new(self.db, &self.ledger_dir, self.policy.clone());
        match writer.refresh_domain_trust() {
            Ok(Some(update)) => {
                errors.extend(update.errors);
                errors.extend(
                    update
                        .unwritten
                        .into_iter()
                        .map(|reason| format!("self_model_not_written:{reason}")),
                );
                Some(update.facts_written > 0)
            }
            Ok(None) => {
                errors.push("self_model_updates_not_permitted_by_policy".into());
                None
            }
            Err(error) => {
                errors.push(format!("refresh_self_model:{error}"));
                None
            }
        }
    }

    fn finish(
        &self,
        mut report: TickReport,
        started: Instant,
    ) -> Result<TickReport, CognitiveError> {
        report.duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        store_tick_report(self.db, &report)?;
        Ok(report)
    }
}

impl TickReport {
    pub fn empty() -> Self {
        Self {
            tick_id: Uuid::new_v4().to_string(),
            dead_letters_replayed: None,
            proposals_evaluated: 0,
            proposals_auto_applied: 0,
            proposals_denied: 0,
            proposals_generated: 0,
            self_model_updated: None,
            errors: Vec::new(),
            duration_ms: 0,
            created_at: Utc::now(),
        }
    }
}

fn recent_proposable_reflections(
    db: &DbInstance,
    errors: &mut Vec<String>,
) -> Vec<ReflectionRecord> {
    let rows = run_script_guarded(
        db,
        "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] := \
         *cognitive_reflections{reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at}",
        Default::default(),
        ScriptMutability::Immutable,
        "query proposable cognitive reflections",
    );
    let Ok(rows) = rows else {
        errors.push("query_proposable_reflections_failed".into());
        return Vec::new();
    };
    rows.rows
        .iter()
        .filter(|row| row[10].get_bool() == Some(true))
        .filter_map(|row| row_to_reflection(row))
        .take(50)
        .collect()
}

fn row_to_reflection(row: &[DataValue]) -> Option<ReflectionRecord> {
    Some(ReflectionRecord {
        reflection_id: str_col(row, 0),
        session_id: str_col(row, 1),
        turn_number: row[2].get_int()?.max(0) as u64,
        decision_id: str_col(row, 3),
        situation_kind: situation_kind(&str_col(row, 4)),
        attempted: str_col(row, 5),
        worked: str_col(row, 6),
        failed: str_col(row, 7),
        outcome: outcome_summary(&str_col(row, 8)),
        lesson: str_col(row, 9),
        should_propose: row[10].get_bool().unwrap_or(false),
        proposed_rule_id: non_empty(str_col(row, 11)),
        created_at: parse_time(&str_col(row, 12)),
    })
}

fn str_col(row: &[DataValue], index: usize) -> String {
    row[index].get_str().unwrap_or("").to_string()
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn parse_time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn situation_kind(value: &str) -> SituationKind {
    match value {
        "ci_debug" => SituationKind::CiDebug,
        "code_change" => SituationKind::CodeChange,
        "git_mutation" => SituationKind::GitMutation,
        "pipeline_control" => SituationKind::PipelineControl,
        "research" => SituationKind::Research,
        "world_model_task" => SituationKind::WorldModelTask,
        "high_risk" => SituationKind::HighRisk,
        "simple_question" => SituationKind::SimpleQuestion,
        "ambiguous" => SituationKind::Ambiguous,
        _ => SituationKind::Greeting,
    }
}

fn outcome_summary(value: &str) -> OutcomeSummary {
    match value {
        "partial_success" => OutcomeSummary::PartialSuccess,
        "user_corrected" => OutcomeSummary::UserCorrected,
        "degraded" => OutcomeSummary::Degraded,
        "success" => OutcomeSummary::Success,
        _ => OutcomeSummary::Failure,
    }
}
