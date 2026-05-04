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
    migrate_gt_runs_cost_usd(db)?;
    ensure_gt_fingerprints(db)?;
    ensure_gt_routing_decisions(db)?;
    ensure_gt_enabled_specialists(db)?;
    ensure_gt_skipped_specialists(db)?;
    ensure_gt_specialist_outputs(db)?;
    ensure_gt_run_checkpoints(db)?;
    ensure_gt_sections(db)?;
    ensure_gt_final_reports(db)?;
    ensure_gt_provenance_edges(db)?;
    ensure_gt_specimen_library(db)?;
    Ok(())
}

fn ensure_gt_run_checkpoints(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_run_checkpoints {
            run_id: String, checkpoint_key: String =>
            checkpoint_type: String,
            status: String,
            detail_json: String default "{}",
            created_at: String,
        }"#,
    )
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
            cost_usd: String default "0.0",
        }"#,
    )
}

fn migrate_gt_runs_cost_usd(db: &DbInstance) -> Result<()> {
    if gt_runs_has_cost_usd(db)? {
        return Ok(());
    }

    let rows = db
        .run_script(
            "?[run_id, situation, started_at, completed_at, status] := \
         *gt_runs{run_id, situation, started_at, completed_at, status}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("snapshot gt_runs before migration failed: {e}"))?;

    db.run_script(
        "{::remove gt_runs}",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("remove old gt_runs relation failed: {e}"))?;
    ensure_gt_runs(db)?;

    for row in rows.rows {
        let mut params = std::collections::BTreeMap::new();
        params.insert("rid".into(), row[0].clone());
        params.insert("sit".into(), row[1].clone());
        params.insert("sa".into(), row[2].clone());
        params.insert("ca".into(), row[3].clone());
        params.insert("st".into(), row[4].clone());
        params.insert("cost".into(), cozo::DataValue::from("0.000000"));

        db.run_script(
            "?[run_id, situation, started_at, completed_at, status, cost_usd] \
             <- [[$rid, $sit, $sa, $ca, $st, $cost]] \
             :put gt_runs { run_id => situation, started_at, completed_at, status, cost_usd }",
            params,
            ScriptMutability::Mutable,
        )
        .map_err(|e| anyhow::anyhow!("reinsert migrated gt_runs row failed: {e}"))?;
    }

    Ok(())
}

fn gt_runs_has_cost_usd(db: &DbInstance) -> Result<bool> {
    match db.run_script(
        "?[cost] := *gt_runs{cost_usd: cost}",
        Default::default(),
        ScriptMutability::Immutable,
    ) {
        Ok(_) => Ok(true),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("cost_usd") {
                Ok(false)
            } else {
                Err(anyhow::anyhow!("failed to inspect gt_runs schema: {msg}"))
            }
        }
    }
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

fn ensure_gt_routing_decisions(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_routing_decisions {
            run_id: String =>
            fingerprint_id: String,
            enabled_specialists_json: String default "[]",
            skipped_specialists_json: String default "[]",
            evaluated_conditions_json: String default "[]",
            created_at: String,
        }"#,
    )
}

fn ensure_gt_enabled_specialists(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_enabled_specialists {
            run_id: String, agent_key: String =>
            mandatory: String default "false",
            condition_evaluated: String default "false",
            depends_on_json: String default "[]",
            created_at: String,
        }"#,
    )
}

fn ensure_gt_skipped_specialists(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_skipped_specialists {
            run_id: String, agent_key: String =>
            reason: String,
            condition_evaluated: String default "false",
            created_at: String,
        }"#,
    )
}

fn ensure_gt_specialist_outputs(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_specialist_outputs {
            run_id: String, agent_key: String =>
            output_json: String default "",
            status: String default "pending",
            started_at: String default "",
            completed_at: String default "",
            duration_ms: String default "0",
            cost_usd: String default "0.0",
        }"#,
    )
}

fn ensure_gt_sections(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_sections {
            run_id: String, section_id: String =>
            section_type: String,
            title: String,
            content_md: String default "",
            source_specialists_json: String default "[]",
            created_at: String,
        }"#,
    )
}

fn ensure_gt_final_reports(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_final_reports {
            run_id: String =>
            report_md: String default "",
            created_at: String,
            total_cost_usd: String default "0.0",
            total_duration_ms: String default "0",
        }"#,
    )
}

fn ensure_gt_provenance_edges(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_provenance_edges {
            edge_id: String =>
            from_artifact_id: String,
            to_artifact_id: String,
            edge_type: String,
            created_at: String,
        }"#,
    )
}

fn ensure_gt_specimen_library(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_specimen_library {
            specimen_id: String =>
            situation_type: String,
            cooperation: String,
            payoff_sum: String,
            symmetry: String,
            timing: String,
            perfect_info: String,
            complete_info: String,
            cardinality: String,
            strategy_space: String,
            horizon: String,
            primary_family: String,
            notes: String default "",
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
        db.run_script(script, Default::default(), ScriptMutability::Mutable)
            .unwrap();

        let script2 = r#"
            ?[run_id, situation, started_at, completed_at, status]
            <- [["run-1", "Test situation updated", "2026-01-01T00:00:00Z", "2026-01-01T00:00:05Z", "completed"]]
            :put gt_runs { run_id => situation, started_at, completed_at, status }
        "#;
        db.run_script(script2, Default::default(), ScriptMutability::Mutable)
            .unwrap();

        let result = db
            .run_script(
                "?[count(run_id)] := *gt_runs{run_id}, run_id = \"run-1\"",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .unwrap();
        assert_eq!(result.rows.len(), 1, ":put must upsert, not duplicate");
    }

    #[test]
    fn test_gt_runs_cost_usd_migration_preserves_existing_rows() {
        let db = test_db();

        db.run_script(
            r#":create gt_runs {
                run_id: String =>
                situation: String,
                started_at: String,
                completed_at: String default "",
                status: String,
            }"#,
            Default::default(),
            ScriptMutability::Mutable,
        )
        .unwrap();
        db.run_script(
            r#"
            ?[run_id, situation, started_at, completed_at, status]
            <- [["run-old", "Old situation", "2026-01-01T00:00:00Z", "", "running"]]
            :put gt_runs { run_id => situation, started_at, completed_at, status }
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        )
        .unwrap();

        ensure_gametheory_schema(&db).unwrap();

        let rows = db
            .run_script(
                "?[status, cost] := *gt_runs{run_id, status, cost_usd: cost}, run_id = \"run-old\"",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .unwrap();
        assert_eq!(rows.rows[0][0].get_str().unwrap(), "running");
        assert_eq!(rows.rows[0][1].get_str().unwrap(), "0.000000");
    }

    #[test]
    fn test_phase4_schema_idempotent() {
        let db = test_db();

        // First creation — must succeed
        ensure_gametheory_schema(&db).unwrap();

        // Second creation — must not panic (all 9 relations already exist)
        ensure_gametheory_schema(&db).unwrap();

        // Verify we can insert into each new relation
        // gt_routing_decisions
        db.run_script(
            r#"
            ?[run_id, fingerprint_id, enabled_specialists_json, skipped_specialists_json, evaluated_conditions_json, created_at]
            <- [["run-1", "fp-1", "[]", "[]", "[]", "2026-05-03T00:00:00Z"]]
            :put gt_routing_decisions { run_id => fingerprint_id, enabled_specialists_json, skipped_specialists_json, evaluated_conditions_json, created_at }
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        ).unwrap();

        // gt_enabled_specialists (composite key)
        db.run_script(
            r#"
            ?[run_id, agent_key, mandatory, condition_evaluated, depends_on_json, created_at]
            <- [["run-1", "gt-nash", "true", "true", '["gt-classify-structure"]', "2026-05-03T00:00:00Z"]]
            :put gt_enabled_specialists { run_id, agent_key => mandatory, condition_evaluated, depends_on_json, created_at }
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        ).unwrap();

        // gt_skipped_specialists
        db.run_script(
            r#"
            ?[run_id, agent_key, reason, condition_evaluated, created_at]
            <- [["run-1", "gt-mixed", "condition false", "true", "2026-05-03T00:00:00Z"]]
            :put gt_skipped_specialists { run_id, agent_key => reason, condition_evaluated, created_at }
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        ).unwrap();

        // gt_specialist_outputs
        db.run_script(
            r#"
            ?[run_id, agent_key, output_json, status, started_at, completed_at, duration_ms, cost_usd]
            <- [["run-1", "gt-nash", '{"result":"ok"}', "completed", "2026-05-03T00:00:00Z", "2026-05-03T00:00:01Z", "1000", "0.0"]]
            :put gt_specialist_outputs { run_id, agent_key => output_json, status, started_at, completed_at, duration_ms, cost_usd }
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        ).unwrap();

        // gt_sections
        db.run_script(
            r#"
            ?[run_id, section_id, section_type, title, content_md, source_specialists_json, created_at]
            <- [["run-1", "sec-1", "ExecutiveSummary", "Executive Summary", "Summary content.", '["gt-nash"]', "2026-05-03T00:00:00Z"]]
            :put gt_sections { run_id, section_id => section_type, title, content_md, source_specialists_json, created_at }
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        ).unwrap();

        // gt_final_reports
        db.run_script(
            r#"
            ?[run_id, report_md, created_at, total_cost_usd, total_duration_ms]
            <- [["run-1", "Final report content.", "2026-05-03T00:00:00Z", "0.0", "5000"]]
            :put gt_final_reports { run_id => report_md, created_at, total_cost_usd, total_duration_ms }
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        ).unwrap();

        // gt_provenance_edges
        db.run_script(
            r#"
            ?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at]
            <- [["edge-1", "run-1", "sec-1", "contains", "2026-05-03T00:00:00Z"]]
            :put gt_provenance_edges { edge_id => from_artifact_id, to_artifact_id, edge_type, created_at }
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        ).unwrap();

        // Verify all relations have at least 1 row each
        let checks: &[(&str, &str)] = &[
            ("gt_routing_decisions", "run_id"),
            ("gt_enabled_specialists", "run_id"),
            ("gt_skipped_specialists", "run_id"),
            ("gt_specialist_outputs", "run_id"),
            ("gt_sections", "run_id"),
            ("gt_final_reports", "run_id"),
            ("gt_provenance_edges", "edge_id"),
        ];
        for &(rel, key_col) in checks {
            let query = format!(
                "?[count({key})] := *{rel}{{{key}}}, {key} = \"run-1\"",
                key = key_col,
                rel = rel,
            );
            // gt_provenance_edges uses edge_id, not run_id
            let query = if rel == "gt_provenance_edges" {
                "?[count(edge_id)] := *gt_provenance_edges{edge_id}, edge_id = \"edge-1\""
                    .to_string()
            } else {
                query
            };
            let result = db
                .run_script(&query, Default::default(), ScriptMutability::Immutable)
                .unwrap();
            assert_eq!(result.rows.len(), 1, "{} must have 1 row", rel);
        }
    }

    #[test]
    fn test_routing_decision_persisted_with_correct_shape() {
        let db = test_db();
        ensure_gametheory_schema(&db).unwrap();

        // Insert routing decision with known JSON arrays (use single-quoted
        // Cozo strings for embedded JSON to avoid escaping issues).
        db.run_script(
            r#"
            ?[run_id, fingerprint_id, enabled_specialists_json, skipped_specialists_json, evaluated_conditions_json, created_at]
            <- [["run-rd-1", "fp-rd-1", '["gt-nash","gt-dominant-strategy"]', '["gt-mixed"]', '[{"expr":"coop","result":true}]', "2026-05-03T00:00:00Z"]]
            :put gt_routing_decisions { run_id => fingerprint_id, enabled_specialists_json, skipped_specialists_json, evaluated_conditions_json, created_at }
            "#,
            Default::default(),
            ScriptMutability::Mutable,
        ).unwrap();

        // Query back by run_id
        let result = db
            .run_script(
                "?[run_id, fingerprint_id, enabled_specialists_json, skipped_specialists_json, evaluated_conditions_json, created_at] \
                 := *gt_routing_decisions{run_id, fingerprint_id, enabled_specialists_json, skipped_specialists_json, evaluated_conditions_json, created_at}, \
                 run_id = \"run-rd-1\"",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .unwrap();

        assert_eq!(result.rows.len(), 1, "must have 1 routing decision row");
        let row = &result.rows[0];
        assert_eq!(row[0].get_str().unwrap(), "run-rd-1");
        assert_eq!(row[1].get_str().unwrap(), "fp-rd-1");
        // JSON arrays roundtrip through Cozo string storage
        let enabled_back: Vec<String> = serde_json::from_str(row[2].get_str().unwrap()).unwrap();
        assert_eq!(enabled_back.len(), 2);
        assert!(enabled_back.contains(&"gt-nash".to_string()));
        assert!(enabled_back.contains(&"gt-dominant-strategy".to_string()));
        let created = row[5].get_str().unwrap();
        assert_eq!(created, "2026-05-03T00:00:00Z");
    }
}
