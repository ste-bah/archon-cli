//! Bridge provider runtime incidents into the agent evolution ledger.

use std::sync::Arc;

use archon_learning::agent_evolution_ledger::{
    AgentPerformanceLedgerRecord, insert_agent_performance_ledger_record,
};
use cozo::DbInstance;

pub(crate) struct ProviderIncidentLedgerInput<'a> {
    pub(crate) db: Option<&'a Arc<DbInstance>>,
    pub(crate) agent_type: Option<&'a str>,
    pub(crate) agent_version: Option<&'a str>,
    pub(crate) run_id: Option<&'a str>,
    pub(crate) model_id: &'a str,
    pub(crate) provider_id: &'a str,
    pub(crate) provider_event_id: &'a str,
    pub(crate) reason_code: &'a str,
}

pub(crate) fn record_provider_incident(input: ProviderIncidentLedgerInput<'_>) {
    let Some(db) = input.db else {
        return;
    };
    let Some(agent_type) = input.agent_type.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    let mut record = AgentPerformanceLedgerRecord::new(
        format!("ledger-{}", uuid::Uuid::new_v4()),
        agent_type,
        "failed",
        chrono::Utc::now().to_rfc3339(),
    )
    .with_model_provider(input.model_id, input.provider_id)
    .with_provider_incident(input.provider_event_id)
    .add_evidence(format!("provider_event:{}", input.provider_event_id))
    .add_evidence(format!(
        "provider_reason:{}",
        sanitized_reason(input.reason_code)
    ));
    record.completion_rate = Some(0.0);
    if let Some(run_id) = input.run_id.filter(|value| !value.trim().is_empty()) {
        record = record.with_run_id(run_id);
    }
    if let Some(version) = input.agent_version.filter(|value| !value.trim().is_empty()) {
        record = record.with_agent_version(version);
    }
    if let Err(error) = insert_agent_performance_ledger_record(db, &record) {
        tracing::warn!(
            %error,
            agent = %agent_type,
            provider = %input.provider_id,
            "provider incident ledger persistence failed"
        );
    }
}

fn sanitized_reason(reason: &str) -> String {
    let normalized: String = reason
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    let trimmed = normalized.trim_matches('_');
    if trimmed.is_empty() {
        "provider_failure".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Arc<DbInstance> {
        let path = format!(
            "/tmp/test-provider-incident-ledger-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        Arc::new(db)
    }

    #[test]
    fn provider_incident_persists_agent_ledger_row() {
        let db = test_db();

        record_provider_incident(ProviderIncidentLedgerInput {
            db: Some(&db),
            agent_type: Some("reviewer"),
            agent_version: Some("1.2.3"),
            run_id: Some("session-1"),
            model_id: "claude-sonnet-4-6",
            provider_id: "anthropic",
            provider_event_id: "provider-event-1",
            reason_code: "rate_limited",
        });

        let rows = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
            &db, "reviewer",
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].completion_status, "failed");
        assert_eq!(
            rows[0].provider_incident_id.as_deref(),
            Some("provider-event-1")
        );
        assert_eq!(rows[0].agent_version.as_deref(), Some("1.2.3"));
        assert_eq!(rows[0].run_id.as_deref(), Some("session-1"));
        assert!(
            rows[0]
                .evidence_ids
                .contains(&"provider_reason:rate_limited".to_string())
        );
    }

    #[test]
    fn missing_agent_type_does_not_persist() {
        let db = test_db();

        record_provider_incident(ProviderIncidentLedgerInput {
            db: Some(&db),
            agent_type: None,
            agent_version: None,
            run_id: Some("session-1"),
            model_id: "model",
            provider_id: "provider",
            provider_event_id: "provider-event-1",
            reason_code: "failure",
        });

        let rows = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
            &db, "main",
        )
        .unwrap();
        assert!(rows.is_empty());
    }
}
