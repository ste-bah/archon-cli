use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentShadowEvaluationRecord {
    pub evaluation_id: String,
    pub proposal_id: String,
    pub agent_type: String,
    pub candidate_version_id: Option<String>,
    pub baseline_version_id: Option<String>,
    pub task_set_id: Option<String>,
    pub baseline_score: f64,
    pub candidate_score: f64,
    pub regression_count: i64,
    pub improvement_count: i64,
    pub verdict: String,
    pub evidence_json: serde_json::Value,
    pub created_at: String,
}

impl AgentShadowEvaluationRecord {
    pub fn new(
        evaluation_id: impl Into<String>,
        proposal_id: impl Into<String>,
        agent_type: impl Into<String>,
        verdict: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            evaluation_id: evaluation_id.into(),
            proposal_id: proposal_id.into(),
            agent_type: agent_type.into(),
            candidate_version_id: None,
            baseline_version_id: None,
            task_set_id: None,
            baseline_score: 0.0,
            candidate_score: 0.0,
            regression_count: 0,
            improvement_count: 0,
            verdict: verdict.into(),
            evidence_json: serde_json::json!({}),
            created_at: created_at.into(),
        }
    }

    pub fn with_versions(
        mut self,
        candidate_version_id: impl Into<String>,
        baseline_version_id: impl Into<String>,
    ) -> Self {
        self.candidate_version_id = Some(candidate_version_id.into());
        self.baseline_version_id = Some(baseline_version_id.into());
        self
    }

    pub fn with_task_set(mut self, task_set_id: impl Into<String>) -> Self {
        self.task_set_id = Some(task_set_id.into());
        self
    }

    pub fn with_scores(mut self, baseline_score: f64, candidate_score: f64) -> Self {
        self.baseline_score = clamp_unit(baseline_score);
        self.candidate_score = clamp_unit(candidate_score);
        self
    }

    pub fn with_counts(mut self, regression_count: i64, improvement_count: i64) -> Self {
        self.regression_count = regression_count.max(0);
        self.improvement_count = improvement_count.max(0);
        self
    }

    pub fn with_evidence_json(mut self, evidence_json: serde_json::Value) -> Self {
        self.evidence_json = evidence_json;
        self
    }
}

pub fn insert_agent_shadow_evaluation(
    db: &DbInstance,
    evaluation: &AgentShadowEvaluationRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert(
        "eid".into(),
        DataValue::from(evaluation.evaluation_id.as_str()),
    );
    params.insert(
        "pid".into(),
        DataValue::from(evaluation.proposal_id.as_str()),
    );
    params.insert(
        "agent".into(),
        DataValue::from(evaluation.agent_type.as_str()),
    );
    params.insert(
        "candidate".into(),
        DataValue::from(evaluation.candidate_version_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "baseline".into(),
        DataValue::from(evaluation.baseline_version_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "taskset".into(),
        DataValue::from(evaluation.task_set_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "base_score".into(),
        DataValue::from(clamp_unit(evaluation.baseline_score)),
    );
    params.insert(
        "cand_score".into(),
        DataValue::from(clamp_unit(evaluation.candidate_score)),
    );
    params.insert(
        "regressions".into(),
        DataValue::from(evaluation.regression_count.max(0)),
    );
    params.insert(
        "improvements".into(),
        DataValue::from(evaluation.improvement_count.max(0)),
    );
    params.insert(
        "verdict".into(),
        DataValue::from(evaluation.verdict.as_str()),
    );
    params.insert(
        "evidence".into(),
        DataValue::from(evaluation.evidence_json.to_string().as_str()),
    );
    params.insert(
        "created".into(),
        DataValue::from(evaluation.created_at.as_str()),
    );

    db.run_script(
        shadow_evaluation_put_script(),
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert agent_shadow_evaluations failed: {e}"))?;
    Ok(())
}

pub fn get_agent_shadow_evaluation(
    db: &DbInstance,
    evaluation_id: &str,
) -> Result<Option<AgentShadowEvaluationRecord>> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(evaluation_id));
    let result = db
        .run_script(
            shadow_evaluation_query("evaluation_id = $eid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get agent_shadow_evaluation failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_shadow_evaluation(row)))
}

pub fn list_agent_shadow_evaluations_by_proposal(
    db: &DbInstance,
    proposal_id: &str,
) -> Result<Vec<AgentShadowEvaluationRecord>> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal_id));
    let result = db
        .run_script(
            shadow_evaluation_query("proposal_id = $pid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list agent_shadow_evaluations by proposal failed: {e}"))?;
    Ok(sorted(
        result.rows.iter().map(|row| row_to_shadow_evaluation(row)),
    ))
}

pub fn list_agent_shadow_evaluations_by_agent(
    db: &DbInstance,
    agent_type: &str,
) -> Result<Vec<AgentShadowEvaluationRecord>> {
    let mut params = BTreeMap::new();
    params.insert("agent".into(), DataValue::from(agent_type));
    let result = db
        .run_script(
            shadow_evaluation_query("agent_type = $agent"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list agent_shadow_evaluations by agent failed: {e}"))?;
    Ok(sorted(
        result.rows.iter().map(|row| row_to_shadow_evaluation(row)),
    ))
}

fn shadow_evaluation_put_script() -> &'static str {
    "?[evaluation_id, proposal_id, agent_type, candidate_version_id, \
     baseline_version_id, task_set_id, baseline_score, candidate_score, \
     regression_count, improvement_count, verdict, evidence_json, created_at] \
     <- [[$eid, $pid, $agent, $candidate, $baseline, $taskset, \
     $base_score, $cand_score, $regressions, $improvements, $verdict, \
     $evidence, $created]] :put agent_shadow_evaluations { evaluation_id => \
     proposal_id, agent_type, candidate_version_id, baseline_version_id, \
     task_set_id, baseline_score, candidate_score, regression_count, \
     improvement_count, verdict, evidence_json, created_at }"
}

fn shadow_evaluation_query(predicate: &'static str) -> &'static str {
    match predicate {
        "evaluation_id = $eid" => {
            "?[evaluation_id, proposal_id, agent_type, candidate_version_id, \
             baseline_version_id, task_set_id, baseline_score, candidate_score, \
             regression_count, improvement_count, verdict, evidence_json, \
             created_at] := *agent_shadow_evaluations{evaluation_id, proposal_id, \
             agent_type, candidate_version_id, baseline_version_id, task_set_id, \
             baseline_score, candidate_score, regression_count, improvement_count, \
             verdict, evidence_json, created_at}, evaluation_id = $eid"
        }
        "proposal_id = $pid" => {
            "?[evaluation_id, proposal_id, agent_type, candidate_version_id, \
             baseline_version_id, task_set_id, baseline_score, candidate_score, \
             regression_count, improvement_count, verdict, evidence_json, \
             created_at] := *agent_shadow_evaluations{evaluation_id, proposal_id, \
             agent_type, candidate_version_id, baseline_version_id, task_set_id, \
             baseline_score, candidate_score, regression_count, improvement_count, \
             verdict, evidence_json, created_at}, proposal_id = $pid"
        }
        _ => {
            "?[evaluation_id, proposal_id, agent_type, candidate_version_id, \
             baseline_version_id, task_set_id, baseline_score, candidate_score, \
             regression_count, improvement_count, verdict, evidence_json, \
             created_at] := *agent_shadow_evaluations{evaluation_id, proposal_id, \
             agent_type, candidate_version_id, baseline_version_id, task_set_id, \
             baseline_score, candidate_score, regression_count, improvement_count, \
             verdict, evidence_json, created_at}, agent_type = $agent"
        }
    }
}

fn row_to_shadow_evaluation(row: &[DataValue]) -> AgentShadowEvaluationRecord {
    AgentShadowEvaluationRecord {
        evaluation_id: str_col(row, 0).to_string(),
        proposal_id: str_col(row, 1).to_string(),
        agent_type: str_col(row, 2).to_string(),
        candidate_version_id: non_empty(str_col(row, 3)),
        baseline_version_id: non_empty(str_col(row, 4)),
        task_set_id: non_empty(str_col(row, 5)),
        baseline_score: row[6].get_float().unwrap_or(0.0),
        candidate_score: row[7].get_float().unwrap_or(0.0),
        regression_count: row[8].get_int().unwrap_or(0),
        improvement_count: row[9].get_int().unwrap_or(0),
        verdict: str_col(row, 10).to_string(),
        evidence_json: serde_json::from_str(str_col(row, 11))
            .unwrap_or_else(|_| serde_json::json!({})),
        created_at: str_col(row, 12).to_string(),
    }
}

fn sorted(
    records: impl Iterator<Item = AgentShadowEvaluationRecord>,
) -> Vec<AgentShadowEvaluationRecord> {
    let mut records: Vec<_> = records.collect();
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    records
}

fn str_col(row: &[DataValue], index: usize) -> &str {
    row[index].get_str().unwrap_or("")
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-agent-shadow-evaluations-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn agent_shadow_evaluation_roundtrips() {
        let db = test_db();
        let evaluation = AgentShadowEvaluationRecord::new(
            "shadow-eval-1",
            "agent-evo-prop-1",
            "reviewer",
            "promote",
            "2026-05-08T12:00:00Z",
        )
        .with_versions("agentv-2", "agentv-1")
        .with_task_set("shadow-suite-1")
        .with_scores(0.62, 0.78)
        .with_counts(1, 7)
        .with_evidence_json(serde_json::json!({"regression_ids": ["task-9"]}));

        insert_agent_shadow_evaluation(&db, &evaluation).unwrap();
        let restored = get_agent_shadow_evaluation(&db, "shadow-eval-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.proposal_id, "agent-evo-prop-1");
        assert_eq!(restored.candidate_version_id.as_deref(), Some("agentv-2"));
        assert_eq!(restored.baseline_score, 0.62);
        assert_eq!(restored.candidate_score, 0.78);
        assert_eq!(restored.evidence_json["regression_ids"][0], "task-9");
    }

    #[test]
    fn shadow_evaluations_list_by_proposal_and_agent() {
        let db = test_db();
        insert_agent_shadow_evaluation(
            &db,
            &AgentShadowEvaluationRecord::new(
                "shadow-eval-1",
                "agent-evo-prop-1",
                "planner",
                "hold",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();
        insert_agent_shadow_evaluation(
            &db,
            &AgentShadowEvaluationRecord::new(
                "shadow-eval-2",
                "agent-evo-prop-1",
                "planner",
                "promote",
                "2026-05-08T12:01:00Z",
            )
            .with_scores(0.55, 0.82),
        )
        .unwrap();

        let by_proposal =
            list_agent_shadow_evaluations_by_proposal(&db, "agent-evo-prop-1").unwrap();
        let by_agent = list_agent_shadow_evaluations_by_agent(&db, "planner").unwrap();

        assert_eq!(by_proposal.len(), 2);
        assert_eq!(by_agent.len(), 2);
        assert_eq!(by_proposal[0].evaluation_id, "shadow-eval-2");
        assert_eq!(by_agent[0].verdict, "promote");
    }
}
