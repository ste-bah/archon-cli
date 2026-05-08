//! CozoDB relation definitions for governed learning.
//!
//! Uses the same idempotent `:create` pattern as `archon-docs::schema`.
//! All `:create` calls use `key => values` syntax.

use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

use crate::errors::COZO_RELATION_ALREADY_EXISTS;

#[cfg(test)]
use crate::errors::COZO_RELATION_NOT_FOUND;

/// Ensure all governed-learning relations exist. Idempotent.
pub fn ensure_learning_schema(db: &DbInstance) -> Result<()> {
    ensure_learning_events(db)?;
    ensure_behaviour_proposals(db)?;
    ensure_behaviour_manifest_versions(db)?;
    ensure_behaviour_policy_decisions(db)?;
    ensure_behaviour_approvals(db)?;
    ensure_provider_runtime_events(db)?;
    ensure_agent_performance_ledger(db)?;
    ensure_agent_evolution_proposals(db)?;
    ensure_agent_profile_versions(db)?;
    ensure_agent_shadow_evaluations(db)?;
    Ok(())
}

/// Run a `:create` script, ignoring "already exists" errors only.
fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if COZO_RELATION_ALREADY_EXISTS
                .iter()
                .any(|phrase| msg.contains(phrase))
            {
                Ok(())
            } else {
                Err(anyhow::anyhow!("learning schema creation failed: {msg}"))
            }
        }
    }
}

fn ensure_learning_events(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create learning_events {
            event_id: String =>
            workspace_id: String,
            event_type: String,
            source_artifact_id: String default "",
            outcome_artifact_id: String default "",
            signal: String default "{}",
            confidence: Float default 0.5,
            provenance_record_id: String default "",
            created_at: String,
        }"#,
    )
}

fn ensure_behaviour_proposals(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create behaviour_proposals {
            proposal_id: String =>
            workspace_id: String,
            manifest_kind: String,
            current_version: String,
            proposed_version: String,
            diff: String,
            evidence_ids_json: String default "[]",
            risk_level: String,
            policy_decision: String,
            status: String,
            created_at: String,
        }"#,
    )
}

fn ensure_behaviour_manifest_versions(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create behaviour_manifest_versions {
            version_id: String =>
            manifest_kind: String,
            version_number: Int default 1,
            content_json: String default "{}",
            diff: String default "",
            parent_version_id: String default "",
            created_by_proposal_id: String default "",
            is_rollback_target: Bool default false,
            created_at: String,
        }"#,
    )
}

fn ensure_behaviour_policy_decisions(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create behaviour_policy_decisions {
            decision_id: String =>
            proposal_id: String,
            rule_name: String,
            outcome: String,
            reason: String,
            evaluated_inputs_json: String default "{}",
            created_at: String,
        }"#,
    )
}

fn ensure_behaviour_approvals(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create behaviour_approvals {
            approval_id: String =>
            proposal_id: String,
            approver: String,
            approved: Bool,
            comment: String default "",
            created_at: String,
        }"#,
    )
}

fn ensure_provider_runtime_events(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create provider_runtime_events {
            event_id: String =>
            provider_id: String,
            profile_id: String default "",
            model_id: String default "",
            runtime_mode: String,
            event_type: String,
            severity: String,
            reason_code: String default "",
            message: String default "",
            retry_count: Int default 0,
            fallback_from: String default "",
            fallback_to: String default "",
            request_id: String default "",
            run_id: String default "",
            pipeline_id: String default "",
            raw_redacted_json: String default "{}",
            created_at: String,
        }"#,
    )
}

fn ensure_agent_performance_ledger(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create agent_performance_ledger {
            event_id: String =>
            agent_type: String,
            agent_version: String default "",
            run_id: String default "",
            pipeline_id: String default "",
            phase: String default "",
            task_hash: String default "",
            model_id: String default "",
            provider_id: String default "",
            profile_id: String default "",
            permission_mode: String default "",
            completion_status: String,
            applied_rate: Float default -1.0,
            completion_rate: Float default -1.0,
            quality_score: Float default -1.0,
            l_score: Float default -1.0,
            user_accepted: String default "",
            user_corrected: String default "",
            gate_failed: String default "",
            test_failed: Bool default false,
            provider_incident_id: String default "",
            evidence_ids_json: String default "[]",
            created_at: String,
        }"#,
    )
}

fn ensure_agent_evolution_proposals(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create agent_evolution_proposals {
            proposal_id: String =>
            agent_type: String,
            current_version: String,
            proposed_version: String,
            kind: String,
            diff: String,
            evidence_ids_json: String default "[]",
            risk_level: String,
            policy_decision: String,
            status: String,
            expected_impact: String default "",
            rollback_target_version: String default "",
            affects_provider_identity: Bool default false,
            affects_permissions: Bool default false,
            created_at: String,
        }"#,
    )
}

fn ensure_agent_profile_versions(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create agent_profile_versions {
            version_id: String =>
            agent_type: String,
            version_number: Int default 1,
            parent_version_id: String default "",
            source: String,
            created_by_proposal_id: String default "",
            profile_json: String default "{}",
            prompt_hash: String default "",
            tools_hash: String default "",
            model_hash: String default "",
            memory_hash: String default "",
            is_active: Bool default false,
            is_rollback_target: Bool default false,
            created_at: String,
        }"#,
    )
}

fn ensure_agent_shadow_evaluations(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create agent_shadow_evaluations {
            evaluation_id: String =>
            proposal_id: String,
            agent_type: String,
            candidate_version_id: String default "",
            baseline_version_id: String default "",
            task_set_id: String default "",
            baseline_score: Float default 0.0,
            candidate_score: Float default 0.0,
            regression_count: Int default 0,
            improvement_count: Int default 0,
            verdict: String,
            evidence_json: String default "{}",
            created_at: String,
        }"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-learning-schema-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    #[test]
    fn test_ensure_schema_idempotent() {
        let db = test_db();
        ensure_learning_schema(&db).expect("first ensure must succeed");
        ensure_learning_schema(&db).expect("second ensure must succeed (idempotent)");
    }

    #[test]
    fn test_relation_not_found_marker() {
        let db = test_db();
        let result = db.run_script(
            "?[event_id] := *nonexistent_xyz{event_id}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        );
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains(COZO_RELATION_NOT_FOUND),
            "Cozo error must contain COZO_RELATION_NOT_FOUND.\nActual: {msg}",
        );
    }
}
