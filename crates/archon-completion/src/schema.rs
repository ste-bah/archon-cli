//! CozoDB schema for completion-integrity relations.
//!
//! Uses the same idempotent `:create` pattern as `archon-docs::schema`
//! and `archon-pipeline::gametheory::schema`.

use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

/// Reuse the same "already exists" markers as archon-docs.
const COZO_RELATION_ALREADY_EXISTS: &[&str] = &["conflicts with an existing", "already exists"];

/// Ensure all completion-integrity relations exist. Idempotent.
pub fn ensure_completion_schema(db: &DbInstance) -> Result<()> {
    ensure_completion_claims(db)?;
    ensure_completion_evidence(db)?;
    ensure_completion_reports(db)?;
    ensure_verification_gate_results(db)?;
    ensure_false_completion_incidents(db)?;
    ensure_agent_model_trust_scores(db)?;
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
                    "completion schema creation failed: {msg}"
                ))
            }
        }
    }
}

fn ensure_completion_claims(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create completion_claims {
            claim_id: String =>
            run_id: String,
            agent_key: String default "",
            model: String default "",
            task_type: String,
            claim_kind: String,
            claim_text: String,
            required_evidence_json: String default "[]",
            linked_evidence_json: String default "[]",
            verified: Bool default false,
            created_at: String,
        }"#,
    )
}

fn ensure_completion_evidence(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create completion_evidence {
            evidence_id: String =>
            run_id: String,
            evidence_kind: String,
            producer: String,
            status: String,
            exit_code: Int default 0,
            input_hash: String default "",
            output_hash: String default "",
            artifact_ids_json: String default "[]",
            provenance_record_id: String default "",
            started_at: String,
            completed_at: String default "",
        }"#,
    )
}

fn ensure_completion_reports(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create completion_reports {
            report_id: String =>
            run_id: String,
            final_state: String,
            claims_json: String default "[]",
            evidence_json: String default "[]",
            failed_gates_json: String default "[]",
            unverified_claims_json: String default "[]",
            calibrated_summary: String,
            provenance_record_id: String default "",
            created_at: String,
        }"#,
    )
}

fn ensure_verification_gate_results(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create verification_gate_results {
            gate_id: String =>
            run_id: String,
            gate_name: String,
            passed: Bool,
            resulting_state: String,
            blocked_claims_json: String default "[]",
            required_missing_evidence_json: String default "[]",
            explanation: String,
            provenance_record_id: String default "",
            created_at: String,
        }"#,
    )
}

fn ensure_false_completion_incidents(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create false_completion_incidents {
            incident_id: String =>
            run_id: String,
            agent_key: String default "",
            model: String default "",
            task_type: String,
            claimed_state: String,
            actual_state: String,
            missing_evidence_json: String default "[]",
            user_correction: String default "",
            severity: String,
            learning_event_id: String default "",
            created_at: String,
        }"#,
    )
}

fn ensure_agent_model_trust_scores(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create agent_model_trust_scores {
            score_id: String =>
            workspace_id: String,
            agent_key: String default "",
            model: String default "",
            task_type: String,
            completion_reliability: Float default 0.5,
            evidence_quality: Float default 0.5,
            false_completion_count: Int default 0,
            verified_completion_count: Int default 0,
            last_updated: String,
        }"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-completion-schema-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    #[test]
    fn test_ensure_schema_idempotent() {
        let db = test_db();
        ensure_completion_schema(&db).expect("first ensure must succeed");
        ensure_completion_schema(&db).expect("second ensure must succeed (idempotent)");
    }
}
