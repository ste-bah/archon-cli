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
    ensure_gt_quality_checks(db)?;
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

fn ensure_gt_quality_checks(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create gt_quality_checks {
            run_id: String, agent_key: String, gate_name: String =>
            passed: String,
            detail: String,
            created_at: String,
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
mod tests;
