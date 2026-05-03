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
                Err(anyhow::anyhow!(
                    "learning schema creation failed: {msg}"
                ))
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
