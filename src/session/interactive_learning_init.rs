use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use cozo::ScriptMutability;

pub(super) struct InitializedDatabases {
    pub(super) pipeline: Option<Arc<cozo::DbInstance>>,
    pub(super) governed: Option<Arc<cozo::DbInstance>>,
}

pub(super) struct SchemaInitialization {
    pub(super) pipeline: bool,
    pub(super) governed: bool,
}

pub(super) struct BlockingInitialization {
    pub(super) db: Arc<cozo::DbInstance>,
    pub(super) schemas: SchemaInitialization,
}

pub(super) async fn initialize(working_dir: &Path) -> InitializedDatabases {
    initialize_with(working_dir, initialize_blocking).await
}

pub(super) async fn initialize_with<Initialize>(
    working_dir: &Path,
    initialize_blocking: Initialize,
) -> InitializedDatabases
where
    Initialize: FnOnce(PathBuf) -> Result<BlockingInitialization> + Send + 'static,
{
    let blocking_working_dir = working_dir.to_path_buf();
    match archon_tui::observability::spawn_blocking_named(
        "interactive-learning-initialize",
        move || initialize_blocking(blocking_working_dir),
    )
    .await
    {
        Ok(Ok(initialized)) => InitializedDatabases {
            pipeline: initialized
                .schemas
                .pipeline
                .then(|| Arc::clone(&initialized.db)),
            governed: initialized.schemas.governed.then_some(initialized.db),
        },
        Ok(Err(error)) => {
            tracing::warn!(
                error = %error,
                "CozoDB learning store unavailable; pipeline persistence and governed runtime evidence disabled"
            );
            InitializedDatabases {
                pipeline: None,
                governed: None,
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "interactive learning initialization join failed; pipeline persistence and governed runtime evidence disabled"
            );
            InitializedDatabases {
                pipeline: None,
                governed: None,
            }
        }
    }
}

fn initialize_blocking(working_dir: PathBuf) -> Result<BlockingInitialization> {
    let db_path = crate::command::store_paths::learning_db_path_for_dir(&working_dir);
    let db = crate::runtime::learning_store::acquire_for_path_with(&db_path, |path| {
        let config = archon_cozo::CozoGuardConfig::for_interactive_db_path(path);
        let db = archon_cozo::open_sqlite_guarded(
            &path.to_string_lossy(),
            "open interactive learning db",
            &config,
        )?;
        archon_learning::cozo_guard::ensure_learning_schema_guarded(&db, path)?;
        Ok(db)
    })?;
    let schemas = SchemaInitialization {
        pipeline: initialize_pipeline_schemas(&working_dir, db.as_ref()),
        governed: true,
    };
    Ok(BlockingInitialization { db, schemas })
}

fn initialize_pipeline_schemas(working_dir: &Path, db: &cozo::DbInstance) -> bool {
    let db_path = crate::command::store_paths::learning_db_path_for_dir(working_dir);
    let guard_config = archon_cozo::CozoGuardConfig::for_db_path(&db_path);
    match archon_cozo::run_guarded(
        "initialize interactive pipeline learning schemas",
        ScriptMutability::Mutable,
        &guard_config,
        || initialize_pipeline_schemas_and_migrate(working_dir, &db_path, db),
    ) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(error = %error, "Learning schema init failed; retrain may not work");
            false
        }
    }
}

#[cfg(test)]
pub(super) fn initialize_schemas(
    working_dir: &Path,
    db: &cozo::DbInstance,
) -> SchemaInitialization {
    let db_path = crate::command::store_paths::learning_db_path_for_dir(working_dir);
    let guard_config = archon_cozo::CozoGuardConfig::for_db_path(&db_path);
    let pipeline = match archon_cozo::run_guarded(
        "initialize interactive pipeline learning schemas",
        ScriptMutability::Mutable,
        &guard_config,
        || initialize_pipeline_schemas_and_migrate(working_dir, &db_path, db),
    ) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(error = %error, "Learning schema init failed; retrain may not work");
            false
        }
    };
    let governed = initialize_governed_schemas(db, &guard_config);
    SchemaInitialization { pipeline, governed }
}

#[cfg(test)]
pub(super) fn initialize_governed_schemas(
    db: &cozo::DbInstance,
    guard_config: &archon_cozo::CozoGuardConfig,
) -> bool {
    match archon_cozo::run_guarded(
        "initialize interactive governed learning schemas",
        ScriptMutability::Mutable,
        &guard_config,
        || archon_learning::schema::ensure_learning_schema(db),
    ) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(error = %error, "governed learning schema init failed; runtime evidence disabled");
            false
        }
    }
}

fn initialize_pipeline_schemas_and_migrate(
    working_dir: &Path,
    db_path: &Path,
    db: &cozo::DbInstance,
) -> Result<()> {
    archon_pipeline::learning::schema::initialize_learning_schemas(db)
        .map_err(|error| anyhow!(error))?;
    crate::command::pipeline_learning_migration::maybe_migrate_legacy_pipeline_learning_with_log(
        working_dir,
        db_path,
        db,
        "interactive",
    );
    Ok(())
}
