use std::panic::{AssertUnwindSafe, catch_unwind};

use cozo::{DbInstance, ScriptMutability};

use crate::types::CognitiveError;

pub const CURRENT_SCHEMA_VERSION: i64 = 1;

pub fn ensure_cognitive_schema(db: &DbInstance) -> Result<(), CognitiveError> {
    for script in SCHEMA_RELATIONS {
        run_idempotent(db, script)?;
    }
    record_schema_version(db)
}

pub fn cognitive_schema_version(db: &DbInstance) -> Result<i64, CognitiveError> {
    let rows = db
        .run_script(
            "?[version] := *cognitive_schema_version{version}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|err| CognitiveError::Schema(err.to_string()))?;
    rows.rows
        .first()
        .and_then(|row| row.first())
        .and_then(|value| value.get_int())
        .ok_or_else(|| CognitiveError::Schema("missing cognitive schema version".into()))
}

const SCHEMA_RELATIONS: &[&str] = &[
    r#":create cognitive_situations {
            situation_id: String =>
            session_id: String,
            turn_number: Int,
            user_text_hash: String,
            surface: String,
            kind: String,
            confidence_score: Float,
            confidence: String,
            evidence_refs: String,
            reason_summary: String,
            created_at: String,
        }"#,
    r#":create cognitive_tool_decisions {
            id: String =>
            situation_id: String,
            session_id: String,
            turn_number: Int,
            tool_name: String,
            verdict_json: String,
            reason: String,
            created_at: String,
        }"#,
    r#":create cognitive_action_candidates {
            candidate_id: String =>
            situation_id: String,
            action_kind: String,
            tool_name: String,
            risk: String,
            expected_evidence: String,
            expected_user_output: String,
            score: Float,
            score_source: String,
            rollback_path: String,
            rejected_reason: String,
            created_at: String,
        }"#,
    r#":create cognitive_decisions {
            decision_id: String =>
            situation_id: String,
            session_id: String,
            turn_number: Int,
            selected_candidate_id: String,
            rejected_candidates_json: String,
            heuristic_scores_json: String,
            policy_verdict_json: String,
            verification_contract_json: String,
            user_visible_summary: String,
            created_at: String,
        }"#,
    r#":create self_model_facts {
            fact_id: String =>
            domain: String,
            fact_kind: String,
            statement: String,
            confidence: Float,
            evidence_count: Int,
            last_seen_at: String,
            expires_at: String,
            created_at: String,
        }"#,
    r#":create cognitive_reflections {
            reflection_id: String =>
            session_id: String,
            turn_number: Int,
            decision_id: String,
            situation_kind: String,
            attempted: String,
            worked: String,
            failed: String,
            outcome: String,
            lesson: String,
            should_propose: Bool,
            proposed_rule_id: String,
            created_at: String,
        }"#,
    r#":create cognitive_prediction_links {
            link_id: String =>
            prediction_id: String,
            situation_id: String,
            decision_id: String,
            candidate_id: String,
            predicted_score: Float,
            actual_outcome: String,
            score_delta: Float,
            created_at: String,
        }"#,
    r#":create cognitive_policy_state {
            state_id: String =>
            rule_name: String,
            decision: String,
            reason: String,
            context_json: String,
            created_at: String,
        }"#,
    r#":create governed_proposals {
            proposal_id: String =>
            reflection_ids_json: String,
            manifest_kind: String,
            risk_level: String,
            evidence_count: Int,
            lesson_tag: String,
            domain: String,
            diff_summary: String,
            rollback_plan: String,
            created_at: String,
        }"#,
    r#":create autonomous_apply_results {
            apply_id: String =>
            proposal_id: String,
            result_kind: String,
            reason: String,
            canary_outcome_ref: String,
            rollback_ref: String,
            created_at: String,
        }"#,
    r#":create canary_outcomes {
            canary_id: String =>
            proposal_id: String,
            passed: Bool,
            details: String,
            snapshot_ref: String,
            created_at: String,
        }"#,
    r#":create cognitive_tick_audit {
            tick_id: String =>
            dead_letters_replayed: Int,
            proposals_evaluated: Int,
            proposals_auto_applied: Int,
            proposals_denied: Int,
            self_model_updated: Bool,
            errors_json: String,
            duration_ms: Int,
            created_at: String,
        }"#,
    r#":create cognitive_schema_version {
            version: Int =>
            created_at: String,
        }"#,
];

fn record_schema_version(db: &DbInstance) -> Result<(), CognitiveError> {
    let created_at = chrono::Utc::now().to_rfc3339();
    let script = format!(
        "?[version, created_at] <- [[{}, '{}']]
         :put cognitive_schema_version {{ version => created_at }}",
        CURRENT_SCHEMA_VERSION, created_at
    );
    run_idempotent(db, script.as_str())
}

fn run_idempotent(db: &DbInstance, script: &str) -> Result<(), CognitiveError> {
    let result = catch_unwind(AssertUnwindSafe(|| {
        db.run_script(script, Default::default(), ScriptMutability::Mutable)
    }));
    match result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => {
            let msg = err.to_string();
            if msg.contains("already exists") || msg.contains("conflicts") {
                Ok(())
            } else {
                Err(CognitiveError::Schema(msg))
            }
        }
        Err(payload) => Err(CognitiveError::Schema(panic_payload_message(
            payload.as_ref(),
        ))),
    }
}

fn panic_payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else {
        "unknown panic payload".to_string()
    }
}
