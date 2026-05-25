use cozo::{DbInstance, ScriptMutability};

use crate::types::CognitiveError;

pub fn ensure_cognitive_schema(db: &DbInstance) -> Result<(), CognitiveError> {
    run_idempotent(
        db,
        r#":create cognitive_situations {
            id: String =>
            session_id: String,
            turn_number: Int,
            user_text_hash: String,
            kind: String,
            confidence_score: Float,
            confidence: String,
            reason: String,
            surface: String,
            created_at: String,
        }"#,
    )?;
    run_idempotent(
        db,
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
    )
}

fn run_idempotent(db: &DbInstance, script: &str) -> Result<(), CognitiveError> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("already exists") || msg.contains("conflicts") {
                Ok(())
            } else {
                Err(CognitiveError::Schema(msg))
            }
        }
    }
}
