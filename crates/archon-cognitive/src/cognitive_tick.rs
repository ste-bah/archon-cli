use std::collections::BTreeSet;
use std::time::Instant;

use archon_policy::CognitivePolicy;
use chrono::{DateTime, Utc};
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::cognitive_tick_store::store_tick_report;
use crate::cozo_guard::{relation_count, run_script_guarded};
use crate::schema::ensure_cognitive_schema;
use crate::{
    CognitiveError, GovernedAutonomousApply, OutcomeSummary, ReflectionRecord, SituationKind,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickReport {
    pub tick_id: String,
    pub dead_letters_replayed: u64,
    pub proposals_evaluated: u64,
    pub proposals_auto_applied: u64,
    pub proposals_denied: u64,
    pub proposals_generated: u64,
    pub self_model_updated: bool,
    pub errors: Vec<String>,
    pub duration_ms: u64,
    pub created_at: DateTime<Utc>,
}

pub struct CognitiveTick<'a> {
    db: &'a DbInstance,
    policy: Option<CognitivePolicy>,
}

impl<'a> CognitiveTick<'a> {
    pub fn new(
        db: &'a DbInstance,
        policy: Option<CognitivePolicy>,
    ) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self { db, policy })
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

    fn replay_dead_letters(&self, _errors: &mut Vec<String>) -> u64 {
        0
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

    fn refresh_self_model(&self, _errors: &mut Vec<String>) -> bool {
        true
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
            dead_letters_replayed: 0,
            proposals_evaluated: 0,
            proposals_auto_applied: 0,
            proposals_denied: 0,
            proposals_generated: 0,
            self_model_updated: false,
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
        .filter_map(row_to_reflection)
        .take(50)
        .collect()
}

fn row_to_reflection(row: &Vec<DataValue>) -> Option<ReflectionRecord> {
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
