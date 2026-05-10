use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentPerformanceLedgerRecord {
    pub event_id: String,
    pub agent_type: String,
    pub agent_version: Option<String>,
    pub run_id: Option<String>,
    pub pipeline_id: Option<String>,
    pub phase: Option<String>,
    pub task_hash: Option<String>,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub profile_id: Option<String>,
    pub permission_mode: Option<String>,
    pub completion_status: String,
    pub applied_rate: Option<f64>,
    pub completion_rate: Option<f64>,
    pub quality_score: Option<f64>,
    pub l_score: Option<f64>,
    pub user_accepted: Option<bool>,
    pub user_corrected: Option<bool>,
    pub gate_failed: Option<String>,
    pub test_failed: bool,
    pub provider_incident_id: Option<String>,
    pub evidence_ids: Vec<String>,
    pub created_at: String,
}

impl AgentPerformanceLedgerRecord {
    pub fn new(
        event_id: impl Into<String>,
        agent_type: impl Into<String>,
        completion_status: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            agent_type: agent_type.into(),
            agent_version: None,
            run_id: None,
            pipeline_id: None,
            phase: None,
            task_hash: None,
            model_id: None,
            provider_id: None,
            profile_id: None,
            permission_mode: None,
            completion_status: completion_status.into(),
            applied_rate: None,
            completion_rate: None,
            quality_score: None,
            l_score: None,
            user_accepted: None,
            user_corrected: None,
            gate_failed: None,
            test_failed: false,
            provider_incident_id: None,
            evidence_ids: Vec::new(),
            created_at: created_at.into(),
        }
    }

    pub fn with_agent_version(mut self, version: impl Into<String>) -> Self {
        self.agent_version = Some(version.into());
        self
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn with_model_provider(
        mut self,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
    ) -> Self {
        self.model_id = Some(model_id.into());
        self.provider_id = Some(provider_id.into());
        self
    }

    pub fn with_scores(mut self, quality_score: Option<f64>, l_score: Option<f64>) -> Self {
        self.quality_score = quality_score.map(clamp_unit);
        self.l_score = l_score.map(clamp_unit);
        self
    }

    pub fn with_user_feedback(mut self, accepted: Option<bool>, corrected: Option<bool>) -> Self {
        self.user_accepted = accepted;
        self.user_corrected = corrected;
        self
    }

    pub fn with_gate_failed(mut self, gate_failed: impl Into<String>) -> Self {
        self.gate_failed = Some(gate_failed.into());
        self
    }

    pub fn with_test_failed(mut self, test_failed: bool) -> Self {
        self.test_failed = test_failed;
        self
    }

    pub fn with_provider_incident(mut self, provider_incident_id: impl Into<String>) -> Self {
        self.provider_incident_id = Some(provider_incident_id.into());
        self
    }

    pub fn add_evidence(mut self, evidence_id: impl Into<String>) -> Self {
        let evidence_id = evidence_id.into();
        if !self.evidence_ids.contains(&evidence_id) {
            self.evidence_ids.push(evidence_id);
        }
        self
    }
}

pub fn insert_agent_performance_ledger_record(
    db: &DbInstance,
    record: &AgentPerformanceLedgerRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(record.event_id.as_str()));
    params.insert("agent".into(), DataValue::from(record.agent_type.as_str()));
    params.insert(
        "ver".into(),
        DataValue::from(record.agent_version.as_deref().unwrap_or("")),
    );
    params.insert(
        "run".into(),
        DataValue::from(record.run_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "pipe".into(),
        DataValue::from(record.pipeline_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "phase".into(),
        DataValue::from(record.phase.as_deref().unwrap_or("")),
    );
    params.insert(
        "task".into(),
        DataValue::from(record.task_hash.as_deref().unwrap_or("")),
    );
    params.insert(
        "model".into(),
        DataValue::from(record.model_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "provider".into(),
        DataValue::from(record.provider_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "profile".into(),
        DataValue::from(record.profile_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "perm".into(),
        DataValue::from(record.permission_mode.as_deref().unwrap_or("")),
    );
    params.insert(
        "status".into(),
        DataValue::from(record.completion_status.as_str()),
    );
    params.insert(
        "applied".into(),
        DataValue::from(score(record.applied_rate)),
    );
    params.insert(
        "completion".into(),
        DataValue::from(score(record.completion_rate)),
    );
    params.insert(
        "quality".into(),
        DataValue::from(score(record.quality_score)),
    );
    params.insert("lscore".into(), DataValue::from(score(record.l_score)));
    params.insert(
        "accepted".into(),
        DataValue::from(optional_bool(record.user_accepted)),
    );
    params.insert(
        "corrected".into(),
        DataValue::from(optional_bool(record.user_corrected)),
    );
    params.insert(
        "gate".into(),
        DataValue::from(record.gate_failed.as_deref().unwrap_or("")),
    );
    params.insert("test".into(), DataValue::from(record.test_failed));
    params.insert(
        "incident".into(),
        DataValue::from(record.provider_incident_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "evidence".into(),
        DataValue::from(serde_json::to_string(&record.evidence_ids)?.as_str()),
    );
    params.insert(
        "created".into(),
        DataValue::from(record.created_at.as_str()),
    );

    db.run_script(ledger_put_script(), params, ScriptMutability::Mutable)
        .map_err(|e| anyhow::anyhow!("insert agent_performance_ledger failed: {e}"))?;
    Ok(())
}

pub fn get_agent_performance_ledger_record(
    db: &DbInstance,
    event_id: &str,
) -> Result<Option<AgentPerformanceLedgerRecord>> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event_id));
    let result = db
        .run_script(
            ledger_query("event_id = $eid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get agent_performance_ledger failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_record(row)))
}

pub fn list_agent_performance_ledger_by_agent(
    db: &DbInstance,
    agent_type: &str,
) -> Result<Vec<AgentPerformanceLedgerRecord>> {
    let mut params = BTreeMap::new();
    params.insert("agent".into(), DataValue::from(agent_type));
    let result = db
        .run_script(
            ledger_query("agent_type = $agent"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list agent_performance_ledger failed: {e}"))?;
    let mut records: Vec<_> = result.rows.iter().map(|row| row_to_record(row)).collect();
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(records)
}

fn ledger_put_script() -> &'static str {
    "?[event_id, agent_type, agent_version, run_id, pipeline_id, phase, \
     task_hash, model_id, provider_id, profile_id, permission_mode, \
     completion_status, applied_rate, completion_rate, quality_score, \
     l_score, user_accepted, user_corrected, gate_failed, test_failed, \
     provider_incident_id, evidence_ids_json, created_at] <- [[$eid, \
     $agent, $ver, $run, $pipe, $phase, $task, $model, $provider, \
     $profile, $perm, $status, $applied, $completion, $quality, $lscore, \
     $accepted, $corrected, $gate, $test, $incident, $evidence, $created]] \
     :put agent_performance_ledger { event_id => agent_type, agent_version, \
     run_id, pipeline_id, phase, task_hash, model_id, provider_id, profile_id, \
     permission_mode, completion_status, applied_rate, completion_rate, \
     quality_score, l_score, user_accepted, user_corrected, gate_failed, \
     test_failed, provider_incident_id, evidence_ids_json, created_at }"
}

fn ledger_query(predicate: &'static str) -> &'static str {
    match predicate {
        "event_id = $eid" => {
            "?[event_id, agent_type, agent_version, run_id, pipeline_id, phase, \
             task_hash, model_id, provider_id, profile_id, permission_mode, \
             completion_status, applied_rate, completion_rate, quality_score, \
             l_score, user_accepted, user_corrected, gate_failed, test_failed, \
             provider_incident_id, evidence_ids_json, created_at] := \
             *agent_performance_ledger{event_id, agent_type, agent_version, \
             run_id, pipeline_id, phase, task_hash, model_id, provider_id, \
             profile_id, permission_mode, completion_status, applied_rate, \
             completion_rate, quality_score, l_score, user_accepted, \
             user_corrected, gate_failed, test_failed, provider_incident_id, \
             evidence_ids_json, created_at}, event_id = $eid"
        }
        _ => {
            "?[event_id, agent_type, agent_version, run_id, pipeline_id, phase, \
             task_hash, model_id, provider_id, profile_id, permission_mode, \
             completion_status, applied_rate, completion_rate, quality_score, \
             l_score, user_accepted, user_corrected, gate_failed, test_failed, \
             provider_incident_id, evidence_ids_json, created_at] := \
             *agent_performance_ledger{event_id, agent_type, agent_version, \
             run_id, pipeline_id, phase, task_hash, model_id, provider_id, \
             profile_id, permission_mode, completion_status, applied_rate, \
             completion_rate, quality_score, l_score, user_accepted, \
             user_corrected, gate_failed, test_failed, provider_incident_id, \
             evidence_ids_json, created_at}, agent_type = $agent"
        }
    }
}

fn row_to_record(row: &[DataValue]) -> AgentPerformanceLedgerRecord {
    AgentPerformanceLedgerRecord {
        event_id: str_col(row, 0).to_string(),
        agent_type: str_col(row, 1).to_string(),
        agent_version: non_empty(str_col(row, 2)),
        run_id: non_empty(str_col(row, 3)),
        pipeline_id: non_empty(str_col(row, 4)),
        phase: non_empty(str_col(row, 5)),
        task_hash: non_empty(str_col(row, 6)),
        model_id: non_empty(str_col(row, 7)),
        provider_id: non_empty(str_col(row, 8)),
        profile_id: non_empty(str_col(row, 9)),
        permission_mode: non_empty(str_col(row, 10)),
        completion_status: str_col(row, 11).to_string(),
        applied_rate: optional_score(row[12].get_float().unwrap_or(-1.0)),
        completion_rate: optional_score(row[13].get_float().unwrap_or(-1.0)),
        quality_score: optional_score(row[14].get_float().unwrap_or(-1.0)),
        l_score: optional_score(row[15].get_float().unwrap_or(-1.0)),
        user_accepted: parse_optional_bool(str_col(row, 16)),
        user_corrected: parse_optional_bool(str_col(row, 17)),
        gate_failed: non_empty(str_col(row, 18)),
        test_failed: row[19].get_bool().unwrap_or(false),
        provider_incident_id: non_empty(str_col(row, 20)),
        evidence_ids: serde_json::from_str(str_col(row, 21)).unwrap_or_default(),
        created_at: str_col(row, 22).to_string(),
    }
}

fn str_col(row: &[DataValue], index: usize) -> &str {
    row[index].get_str().unwrap_or("")
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn score(value: Option<f64>) -> f64 {
    value.map(clamp_unit).unwrap_or(-1.0)
}

fn optional_score(value: f64) -> Option<f64> {
    (value >= 0.0).then_some(value)
}

fn optional_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "",
    }
}

fn parse_optional_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-agent-performance-ledger-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn agent_performance_ledger_roundtrips() {
        let db = test_db();
        let record = AgentPerformanceLedgerRecord::new(
            "ledger-1",
            "reviewer",
            "succeeded",
            "2026-05-08T12:00:00Z",
        )
        .with_agent_version("agentv-2")
        .with_run_id("run-1")
        .with_model_provider("claude-sonnet-4-6", "anthropic")
        .with_scores(Some(1.5), Some(-1.0))
        .with_user_feedback(Some(true), Some(false))
        .add_evidence("provider-event-1")
        .add_evidence("provider-event-1");

        insert_agent_performance_ledger_record(&db, &record).unwrap();
        let restored = get_agent_performance_ledger_record(&db, "ledger-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.agent_type, "reviewer");
        assert_eq!(restored.agent_version.as_deref(), Some("agentv-2"));
        assert_eq!(restored.quality_score, Some(1.0));
        assert_eq!(restored.l_score, Some(0.0));
        assert_eq!(restored.user_accepted, Some(true));
        assert_eq!(restored.evidence_ids, vec!["provider-event-1"]);
    }

    #[test]
    fn agent_performance_ledger_lists_by_agent_newest_first() {
        let db = test_db();
        insert_agent_performance_ledger_record(
            &db,
            &AgentPerformanceLedgerRecord::new(
                "ledger-1",
                "planner",
                "failed",
                "2026-05-08T12:00:00Z",
            )
            .with_gate_failed("tests")
            .with_test_failed(true),
        )
        .unwrap();
        insert_agent_performance_ledger_record(
            &db,
            &AgentPerformanceLedgerRecord::new(
                "ledger-2",
                "planner",
                "succeeded",
                "2026-05-08T12:01:00Z",
            ),
        )
        .unwrap();
        insert_agent_performance_ledger_record(
            &db,
            &AgentPerformanceLedgerRecord::new(
                "ledger-3",
                "coder",
                "succeeded",
                "2026-05-08T12:02:00Z",
            ),
        )
        .unwrap();

        let planner = list_agent_performance_ledger_by_agent(&db, "planner").unwrap();

        assert_eq!(planner.len(), 2);
        assert_eq!(planner[0].event_id, "ledger-2");
        assert_eq!(planner[1].gate_failed.as_deref(), Some("tests"));
        assert!(planner[1].test_failed);
    }
}
