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
    let db = crate::runtime::learning_store::acquire_for_path_with_arc(&db_path, |path| {
        open_interactive_learning_db(path)
    })?;
    let schemas = SchemaInitialization {
        pipeline: initialize_pipeline_schemas(&working_dir, db.as_ref()),
        governed: true,
    };
    Ok(BlockingInitialization { db, schemas })
}

fn open_interactive_learning_db(path: &Path) -> Result<Arc<cozo::DbInstance>> {
    let config = archon_cozo::CozoGuardConfig::for_interactive_db_path(path);
    let db = archon_cozo::open_sqlite_guarded_instance(
        &path.to_string_lossy(),
        "open interactive learning db",
        config,
    )?
    .db_arc();
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

fn initialize_pipeline_schemas(working_dir: &Path, db: &cozo::DbInstance) -> bool {
    match run_pipeline_schema_initialization(working_dir, db) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(error = %error, "Learning schema init failed; retrain may not work");
            false
        }
    }
}

fn run_pipeline_schema_initialization(working_dir: &Path, db: &cozo::DbInstance) -> Result<()> {
    let db_path = crate::command::store_paths::learning_db_path_for_dir(working_dir);
    let guard_config =
        archon_cozo::bound_guard_config(db, "initialize interactive pipeline learning schemas")?;
    archon_cozo::run_guarded(
        "initialize interactive pipeline learning schemas",
        ScriptMutability::Mutable,
        &guard_config,
        || initialize_pipeline_schemas_and_migrate(working_dir, &db_path, db),
    )
}

#[cfg(test)]
pub(super) fn initialize_schemas(
    working_dir: &Path,
    db: &cozo::DbInstance,
) -> SchemaInitialization {
    let pipeline = match run_pipeline_schema_initialization(working_dir, db) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(error = %error, "Learning schema init failed; retrain may not work");
            false
        }
    };
    let db_path = crate::command::store_paths::learning_db_path_for_dir(working_dir);
    let guard_config = archon_cozo::CozoGuardConfig::for_db_path(&db_path);
    let governed = initialize_governed_schemas(db, &guard_config);
    SchemaInitialization { pipeline, governed }
}

#[cfg(test)]
pub(super) fn initialize_governed_schemas(
    db: &cozo::DbInstance,
    _guard_config: &archon_cozo::CozoGuardConfig,
) -> bool {
    match archon_learning::schema::ensure_learning_schema(db) {
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
