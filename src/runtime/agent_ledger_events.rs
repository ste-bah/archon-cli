//! Agent performance ledger bridge for real runtime events.

use std::sync::Arc;

use archon_learning::agent_evolution_ledger::{
    AgentPerformanceLedgerRecord, insert_agent_performance_ledger_record,
};
use cozo::DbInstance;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AgentLedgerContext {
    pub agent_type: String,
    pub agent_version: Option<String>,
    pub session_id: String,
    pub model_id: String,
    pub provider_id: String,
}

impl AgentLedgerContext {
    pub(crate) fn new(
        agent_type: impl Into<String>,
        session_id: impl Into<String>,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
    ) -> Self {
        Self {
            agent_type: agent_type.into(),
            agent_version: None,
            session_id: session_id.into(),
            model_id: model_id.into(),
            provider_id: provider_id.into(),
        }
    }

    pub(crate) fn with_version(mut self, version: Option<String>) -> Self {
        self.agent_version = version.filter(|value| !value.trim().is_empty());
        self
    }
}

pub(crate) fn record_agent_turn_completed(
    db: Option<&Arc<DbInstance>>,
    context: &AgentLedgerContext,
    permission_mode: &str,
    input_tokens: u64,
    output_tokens: u64,
) {
    let mut record = base_record(context, "succeeded", permission_mode);
    record.completion_rate = Some(1.0);
    record = record.add_evidence(format!(
        "turn_usage:input={input_tokens}:output={output_tokens}"
    ));
    persist(db, record);
}

pub(crate) fn record_agent_runtime_error(
    db: Option<&Arc<DbInstance>>,
    context: &AgentLedgerContext,
    permission_mode: &str,
) {
    let mut record = base_record(context, "failed", permission_mode);
    record.completion_rate = Some(0.0);
    record.gate_failed = Some("runtime_error".into());
    persist(db, record);
}

fn base_record(
    context: &AgentLedgerContext,
    completion_status: &str,
    permission_mode: &str,
) -> AgentPerformanceLedgerRecord {
    let mut record = AgentPerformanceLedgerRecord::new(
        format!("ledger-{}", uuid::Uuid::new_v4()),
        context.agent_type.clone(),
        completion_status,
        chrono::Utc::now().to_rfc3339(),
    )
    .with_run_id(context.session_id.clone())
    .with_model_provider(context.model_id.clone(), context.provider_id.clone());
    record.permission_mode = Some(permission_mode.to_string());
    if let Some(version) = &context.agent_version {
        record = record.with_agent_version(version.clone());
    }
    record
}

fn persist(db: Option<&Arc<DbInstance>>, record: AgentPerformanceLedgerRecord) {
    let Some(db) = db else {
        return;
    };
    if let Err(error) = insert_agent_performance_ledger_record(db, &record) {
        tracing::warn!(
            %error,
            agent = %record.agent_type,
            status = %record.completion_status,
            "agent performance ledger persistence failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-runtime-agent-ledger-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn turn_completion_persists_successful_ledger_row() {
        let db = Arc::new(test_db());
        let context =
            AgentLedgerContext::new("reviewer", "session-1", "claude-sonnet-4-6", "anthropic")
                .with_version(Some("1.2".into()));

        record_agent_turn_completed(Some(&db), &context, "default", 10, 20);

        let rows = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
            &db, "reviewer",
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].completion_status, "succeeded");
        assert_eq!(rows[0].completion_rate, Some(1.0));
        assert_eq!(rows[0].agent_version.as_deref(), Some("1.2"));
        assert_eq!(rows[0].permission_mode.as_deref(), Some("default"));
        assert_eq!(rows[0].evidence_ids[0], "turn_usage:input=10:output=20");
    }

    #[test]
    fn runtime_error_persists_failed_ledger_row() {
        let db = Arc::new(test_db());
        let context = AgentLedgerContext::new("main", "session-1", "gpt-5.4", "openai-codex");

        record_agent_runtime_error(Some(&db), &context, "auto");

        let rows = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
            &db, "main",
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].completion_status, "failed");
        assert_eq!(rows[0].completion_rate, Some(0.0));
        assert_eq!(rows[0].gate_failed.as_deref(), Some("runtime_error"));
    }
}
