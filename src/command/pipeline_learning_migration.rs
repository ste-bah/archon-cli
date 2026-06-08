use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

const LEGACY_DB_NAME: &str = "learning.db";
const MIGRATION_MARKER: &str = "learning.db.migrated-to-archon-data";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PipelineLearningMigrationReport {
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub rows_copied: usize,
    pub rows_skipped: usize,
}

struct RelationSpec {
    name: &'static str,
    keys: &'static [&'static str],
    columns: &'static [&'static str],
}

const RELATIONS: &[RelationSpec] = &[
    rel(
        "trajectories",
        &["trajectory_id"],
        &[
            "trajectory_id",
            "route",
            "agent_key",
            "session_id",
            "patterns",
            "context",
            "embedding",
            "quality",
            "reward",
            "feedback_score",
            "weights_path",
            "created_at",
            "updated_at",
        ],
    ),
    rel(
        "trajectory_steps",
        &["step_id"],
        &[
            "step_id",
            "trajectory_id",
            "step_index",
            "action",
            "observation",
            "reward",
            "timestamp",
        ],
    ),
    rel(
        "patterns",
        &["pattern_id"],
        &[
            "pattern_id",
            "pattern_type",
            "description",
            "embedding",
            "frequency",
            "confidence",
            "created_at",
            "updated_at",
        ],
    ),
    rel(
        "causal_nodes",
        &["node_id"],
        &[
            "node_id",
            "label",
            "node_type",
            "probability",
            "evidence_count",
            "created_at",
        ],
    ),
    rel(
        "causal_links",
        &["link_id"],
        &[
            "link_id",
            "source_ids",
            "target_id",
            "strength",
            "link_type",
            "created_at",
        ],
    ),
    rel(
        "provenance_sources",
        &["source_id"],
        &[
            "source_id",
            "source_type",
            "uri",
            "trust_score",
            "last_verified",
            "created_at",
        ],
    ),
    rel(
        "provenance_records",
        &["record_id"],
        &[
            "record_id",
            "source_id",
            "entity_id",
            "entity_type",
            "derivation_chain",
            "confidence",
            "created_at",
        ],
    ),
    rel(
        "desc_episodes",
        &["episode_id"],
        &[
            "episode_id",
            "session_id",
            "description",
            "outcome",
            "reward",
            "tags",
            "created_at",
        ],
    ),
    rel(
        "desc_episode_metadata",
        &["episode_id"],
        &[
            "episode_id",
            "task_type",
            "solution",
            "quality_score",
            "trajectory_id",
            "updated_at",
        ],
    ),
    rel(
        "gnn_weights",
        &["layer_id", "version"],
        &[
            "layer_id",
            "version",
            "in_dim",
            "out_dim",
            "initialization",
            "seed",
            "weights_blob",
            "bias_blob",
            "norm_l2",
            "has_nan",
            "saved_at_ms",
        ],
    ),
    rel(
        "gnn_adam_state",
        &["layer_id", "version"],
        &["layer_id", "version", "m_blob", "v_blob", "step"],
    ),
    rel(
        "gnn_training_runs",
        &["run_id"],
        &[
            "run_id",
            "started_at_ms",
            "completed_at_ms",
            "trigger_reason",
            "samples_processed",
            "epochs_completed",
            "final_loss",
            "best_loss",
            "weight_version_before",
            "weight_version_after",
            "rolled_back",
            "error",
        ],
    ),
    rel(
        "shadow_documents",
        &["doc_id"],
        &[
            "doc_id",
            "original_id",
            "shadow_type",
            "content",
            "metadata",
            "created_at",
            "updated_at",
        ],
    ),
    rel(
        "learning_schema_version",
        &["component"],
        &["component", "version", "applied_at"],
    ),
];

const fn rel(
    name: &'static str,
    keys: &'static [&'static str],
    columns: &'static [&'static str],
) -> RelationSpec {
    RelationSpec {
        name,
        keys,
        columns,
    }
}

pub(crate) fn maybe_migrate_legacy_pipeline_learning(
    cwd: &Path,
    target_path: &Path,
    target: &DbInstance,
) -> Result<Option<PipelineLearningMigrationReport>> {
    if learning_db_override_is_set() {
        return Ok(None);
    }

    let source_path = legacy_pipeline_learning_path(cwd);
    if !source_path.exists() || same_path(&source_path, target_path) {
        return Ok(None);
    }

    let marker_path = cwd.join(".archon").join(MIGRATION_MARKER);
    if marker_path.exists() && target_has_pipeline_rows(target)? {
        return Ok(None);
    }

    let source_path_str = source_path.to_string_lossy().to_string();
    let source = DbInstance::new("newrocksdb", &source_path_str, "")
        .map_err(|error| anyhow::anyhow!("open legacy pipeline learning DB: {error}"))?;
    archon_pipeline::learning::schema::initialize_learning_schemas(&source)
        .context("initialise legacy pipeline learning schema")?;

    let mut rows_copied = 0;
    let mut rows_skipped = 0;
    for spec in RELATIONS {
        let outcome = copy_relation(&source, target, spec)
            .with_context(|| format!("migrate relation {}", spec.name))?;
        rows_copied += outcome.copied;
        rows_skipped += outcome.skipped;
    }

    if let Some(parent) = marker_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        &marker_path,
        format!(
            "source={}\ntarget={}\nrows_copied={rows_copied}\nrows_skipped={rows_skipped}\n",
            source_path.display(),
            target_path.display()
        ),
    )?;

    Ok(Some(PipelineLearningMigrationReport {
        source_path,
        target_path: target_path.to_path_buf(),
        rows_copied,
        rows_skipped,
    }))
}

pub(crate) fn maybe_migrate_legacy_pipeline_learning_with_log(
    cwd: &Path,
    target_path: &Path,
    target: &DbInstance,
    caller: &'static str,
) {
    match maybe_migrate_legacy_pipeline_learning(cwd, target_path, target) {
        Ok(Some(report)) => tracing::info!(
            caller,
            source = %report.source_path.display(),
            target = %report.target_path.display(),
            rows_copied = report.rows_copied,
            rows_skipped = report.rows_skipped,
            "migrated legacy RocksDB learning store"
        ),
        Ok(None) => {}
        Err(error) => tracing::warn!(caller, %error, "legacy learning store migration failed"),
    }
}

fn learning_db_override_is_set() -> bool {
    std::env::var_os("ARCHON_LEARNING_DB_PATH").is_some_and(|value| !value.is_empty())
}

fn legacy_pipeline_learning_path(cwd: &Path) -> PathBuf {
    cwd.join(".archon").join(LEGACY_DB_NAME)
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn target_has_pipeline_rows(target: &DbInstance) -> Result<bool> {
    for spec in RELATIONS
        .iter()
        .filter(|spec| spec.name != "learning_schema_version")
    {
        if !read_relation(target, spec)?.rows.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

struct CopyOutcome {
    copied: usize,
    skipped: usize,
}

fn copy_relation(
    source: &DbInstance,
    target: &DbInstance,
    spec: &RelationSpec,
) -> Result<CopyOutcome> {
    let source_rows = read_relation(source, spec)?;
    if source_rows.rows.is_empty() {
        return Ok(CopyOutcome {
            copied: 0,
            skipped: 0,
        });
    }

    let existing_keys = relation_keys(target, spec)?;
    let mut copied = 0;
    let mut skipped = 0;
    for row in source_rows.rows {
        if existing_keys.contains(&row_key(spec, &row)) {
            skipped += 1;
            continue;
        }
        put_relation_row(target, spec, &row)?;
        copied += 1;
    }
    Ok(CopyOutcome { copied, skipped })
}

fn read_relation(db: &DbInstance, spec: &RelationSpec) -> Result<NamedRows> {
    let columns = spec.columns.join(", ");
    let script = format!("?[{columns}] := *{}{{{columns}}}", spec.name);
    db.run_script(&script, Default::default(), ScriptMutability::Immutable)
        .map_err(|error| anyhow::anyhow!("{error}"))
}

fn relation_keys(db: &DbInstance, spec: &RelationSpec) -> Result<BTreeSet<String>> {
    let rows = read_relation(db, spec)?;
    Ok(rows.rows.iter().map(|row| row_key(spec, row)).collect())
}

fn row_key(spec: &RelationSpec, row: &[DataValue]) -> String {
    (0..spec.keys.len())
        .map(|index| format!("{:?}", row[index]))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

fn put_relation_row(db: &DbInstance, spec: &RelationSpec, row: &[DataValue]) -> Result<()> {
    let params = row
        .iter()
        .enumerate()
        .map(|(index, value)| (format!("p{index}"), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let columns = spec.columns.join(", ");
    let values = (0..spec.columns.len())
        .map(|index| format!("$p{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let key_count = spec.keys.len();
    let value_columns = spec.columns[key_count..].join(", ");
    let script = format!(
        "?[{columns}] <- [[{values}]] :put {} {{ {} => {value_columns} }}",
        spec.name,
        spec.keys.join(", ")
    );
    db.run_script(&script, params, ScriptMutability::Mutable)
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_rocksdb_rows_into_sqlite_learning_store_once() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cwd = temp.path();
        let source_path = legacy_pipeline_learning_path(cwd);
        let target_path = cwd.join(".archon").join("archon-data.db");

        {
            let source =
                DbInstance::new("newrocksdb", source_path.to_str().unwrap(), "").expect("source");
            archon_pipeline::learning::schema::initialize_learning_schemas(&source)
                .expect("source schema");
            insert_trajectory(&source, "traj-legacy", 0.75);
        }

        let target = DbInstance::new("sqlite", target_path.to_str().unwrap(), "").expect("target");
        archon_pipeline::learning::schema::initialize_learning_schemas(&target)
            .expect("target schema");

        let first = maybe_migrate_legacy_pipeline_learning(cwd, &target_path, &target)
            .expect("first migration")
            .expect("report");
        assert!(first.rows_copied > 0);
        assert_eq!(count_trajectories(&target), 1);

        let second = maybe_migrate_legacy_pipeline_learning(cwd, &target_path, &target)
            .expect("second migration");
        assert!(second.is_none());
        assert_eq!(count_trajectories(&target), 1);
    }

    fn insert_trajectory(db: &DbInstance, id: &str, quality: f64) {
        let mut params = BTreeMap::new();
        params.insert("id".into(), DataValue::from(id));
        params.insert("route".into(), DataValue::from("route"));
        params.insert("agent".into(), DataValue::from("agent"));
        params.insert("session".into(), DataValue::from("session"));
        params.insert(
            "patterns".into(),
            DataValue::List(vec![DataValue::from("p")]),
        );
        params.insert(
            "context".into(),
            DataValue::List(vec![DataValue::from("c")]),
        );
        params.insert(
            "embedding".into(),
            DataValue::List(vec![DataValue::from(0.25)]),
        );
        params.insert("quality".into(), DataValue::from(quality));
        params.insert("reward".into(), DataValue::from(1.0));
        params.insert("feedback".into(), DataValue::from(0.5));
        params.insert("weights".into(), DataValue::from("/weights.bin"));
        params.insert("created".into(), DataValue::from(1_i64));
        params.insert("updated".into(), DataValue::from(2_i64));
        db.run_script(
            "?[trajectory_id, route, agent_key, session_id, patterns, context, embedding, quality, reward, feedback_score, weights_path, created_at, updated_at] <- \
             [[$id, $route, $agent, $session, $patterns, $context, $embedding, $quality, $reward, $feedback, $weights, $created, $updated]] \
             :put trajectories { trajectory_id => route, agent_key, session_id, patterns, context, embedding, quality, reward, feedback_score, weights_path, created_at, updated_at }",
            params,
            ScriptMutability::Mutable,
        )
        .expect("insert trajectory");
    }

    fn count_trajectories(db: &DbInstance) -> i64 {
        db.run_script(
            "?[count(trajectory_id)] := *trajectories[trajectory_id, _, _, _, _, _, _, _, _, _, _, _, _]",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .expect("count")
        .rows[0][0]
            .get_int()
            .expect("count int")
    }
}
