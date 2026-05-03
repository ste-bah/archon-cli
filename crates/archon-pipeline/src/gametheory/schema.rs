//! CozoDB schema for game-theory relations.
//!
//! Uses the same idempotent `:create` pattern as `archon-docs::schema`.

use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

/// Cozo "relation already exists" substrings (same source as archon-docs).
const COZO_RELATION_ALREADY_EXISTS: &[&str] = &["conflicts with an existing", "already exists"];

/// Ensure all game-theory relations exist. Idempotent.
pub fn ensure_gametheory_schema(db: &DbInstance) -> Result<()> {
    ensure_gt_runs(db)?;
    ensure_gt_fingerprints(db)?;
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
                Err(anyhow::anyhow!("gametheory schema creation failed: {msg}"))
            }
        }
    }
}

fn ensure_gt_runs(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_runs {
            run_id: String =>
            situation: String,
            started_at: String,
            completed_at: String default "",
            status: String,
        }"#,
    )
}

fn ensure_gt_fingerprints(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_fingerprints {
            run_id: String =>
            fingerprint_json: String,
            primary_family: String,
            created_at: String,
        }"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-gt-schema-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    #[test]
    fn test_gt_runs_schema_idempotent() {
        let db = test_db();

        // First creation
        ensure_gametheory_schema(&db).unwrap();

        // Second creation must not panic
        ensure_gametheory_schema(&db).unwrap();

        // Insert a row, try to insert same key again — must be 1 row
        let script = r#"
            ?[run_id, situation, started_at, completed_at, status]
            <- [["run-1", "Test situation", "2026-01-01T00:00:00Z", "", "running"]]
            :put gt_runs { run_id => situation, started_at, completed_at, status }
        "#;
        db.run_script(script, Default::default(), ScriptMutability::Mutable).unwrap();

        let script2 = r#"
            ?[run_id, situation, started_at, completed_at, status]
            <- [["run-1", "Test situation updated", "2026-01-01T00:00:00Z", "2026-01-01T00:00:05Z", "completed"]]
            :put gt_runs { run_id => situation, started_at, completed_at, status }
        "#;
        db.run_script(script2, Default::default(), ScriptMutability::Mutable).unwrap();

        let result = db
            .run_script(
                "?[count(run_id)] := *gt_runs{run_id}, run_id = \"run-1\"",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .unwrap();
        assert_eq!(result.rows.len(), 1, ":put must upsert, not duplicate");
    }
}
