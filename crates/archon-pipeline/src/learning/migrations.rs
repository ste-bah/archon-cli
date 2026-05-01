//! Schema migration runner for learning relations.
//!
//! CozoDB has no ALTER RELATION. Adding a column requires row-by-row migration
//! in Rust: SELECT old rows, :rm old, :create new, :put rows back.
//!
//! Version sentinel stored in `learning_schema_version` relation prevents
//! re-migration on every restart.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn run_mutable(db: &DbInstance, script: &str) -> Result<NamedRows> {
    db.run_script(script, Default::default(), ScriptMutability::Mutable)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn run_immutable(db: &DbInstance, script: &str) -> Result<NamedRows> {
    db.run_script(script, Default::default(), ScriptMutability::Immutable)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

fn run_mutable_params(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
) -> Result<NamedRows> {
    db.run_script(script, params, ScriptMutability::Mutable)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run any pending migrations for learning schemas.
///
/// Called from `initialize_learning_schemas` after all CREATEs succeed.
/// Idempotent: checks version sentinel before running each migration.
pub fn apply_pending_migrations(db: &DbInstance) -> Result<()> {
    let current = read_trajectories_version(db)?;
    match current {
        None => {
            // Fresh DB — record v2 (current schema) directly, no migration needed.
            record_version(db, "trajectories", 2)?;
        }
        Some(1) => {
            migrate_trajectories_v1_to_v2(db)?;
        }
        Some(2) => {
            // Already current — nothing to do.
        }
        Some(v) if v > 2 => {
            anyhow::bail!(
                "trajectories schema version {v} is newer than this archon-cli binary (expects v2). \
                 Upgrade archon-cli to use this learning database."
            );
        }
        Some(_) => {
            // Unknown version — migrate forward to v2.
            migrate_trajectories_v1_to_v2(db)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Migration logic
// ---------------------------------------------------------------------------

fn migrate_trajectories_v1_to_v2(db: &DbInstance) -> Result<()> {
    // Snapshot all rows from the v1 (12-col) relation before we touch it.
    let old_rows = run_immutable(
        db,
        "?[trajectory_id, route, agent_key, session_id, patterns, context, quality, \
         reward, feedback_score, weights_path, created_at, updated_at] := \
         *trajectories[trajectory_id, route, agent_key, session_id, patterns, context, \
         quality, reward, feedback_score, weights_path, created_at, updated_at]",
    )?;

    // Drop old relation and re-create with the 13-col v2 schema.
    // We use {::remove ...} (system utility) to drop, then :create to re-create.
    run_mutable(db, "{::remove trajectories}")?;
    run_mutable(
        db,
        ":create trajectories { trajectory_id: String => \
         route: String, agent_key: String, session_id: String, \
         patterns: [String], context: [String], embedding: [Float], \
         quality: Float, reward: Float, feedback_score: Float, \
         weights_path: String, created_at: Int, updated_at: Int }",
    )?;

    // Re-insert every row with an empty embedding sentinel.
    for row in old_rows.rows {
        let mut params = BTreeMap::new();
        params.insert("trajectory_id".to_string(), row[0].clone());
        params.insert("route".to_string(), row[1].clone());
        params.insert("agent_key".to_string(), row[2].clone());
        params.insert("session_id".to_string(), row[3].clone());
        params.insert("patterns".to_string(), row[4].clone());
        params.insert("context".to_string(), row[5].clone());
        params.insert("embedding".to_string(), DataValue::List(vec![]));
        params.insert("quality".to_string(), row[6].clone());
        params.insert("reward".to_string(), row[7].clone());
        params.insert("feedback_score".to_string(), row[8].clone());
        params.insert("weights_path".to_string(), row[9].clone());
        params.insert("created_at".to_string(), row[10].clone());
        params.insert("updated_at".to_string(), row[11].clone());
        run_mutable_params(
            db,
            "?[trajectory_id, route, agent_key, session_id, patterns, context, embedding, \
             quality, reward, feedback_score, weights_path, created_at, updated_at] <- \
             [[$trajectory_id, $route, $agent_key, $session_id, $patterns, $context, \
             $embedding, $quality, $reward, $feedback_score, $weights_path, \
             $created_at, $updated_at]] \
             :put trajectories { trajectory_id => route, agent_key, session_id, \
             patterns, context, embedding, quality, reward, feedback_score, \
             weights_path, created_at, updated_at }",
            params,
        )?;
    }

    record_version(db, "trajectories", 2)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Version helpers
// ---------------------------------------------------------------------------

/// Read the current version of the trajectories relation from the sentinel table.
fn read_trajectories_version(db: &DbInstance) -> Result<Option<u32>> {
    let result = run_immutable(
        db,
        "?[v] := *learning_schema_version{component: \"trajectories\", version: v}",
    )?;
    if result.rows.is_empty() {
        return Ok(None);
    }
    let v = result.rows[0][0].get_int().unwrap_or(0) as u32;
    Ok(Some(v))
}

/// Persist (component, version, applied_at=now) to learning_schema_version.
fn record_version(db: &DbInstance, component: &str, version: u32) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    let mut params = BTreeMap::new();
    params.insert("component".to_string(), DataValue::Str(component.into()));
    params.insert("version".to_string(), DataValue::from(version as i64));
    params.insert("applied_at".to_string(), DataValue::from(now));
    run_mutable_params(
        db,
        "?[component, version, applied_at] <- [[$component, $version, $applied_at]] \
         :put learning_schema_version { component => version, applied_at }",
        params,
    )?;
    Ok(())
}
