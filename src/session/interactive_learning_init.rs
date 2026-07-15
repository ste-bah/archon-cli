use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

pub(super) struct InitializedDatabases {
    pub(super) pipeline: Option<Arc<cozo::DbInstance>>,
    pub(super) governed: Option<Arc<cozo::DbInstance>>,
}

pub(super) struct SchemaInitialization {
    pub(super) pipeline: bool,
    pub(super) governed: bool,
}

pub(super) async fn initialize(working_dir: &Path) -> InitializedDatabases {
    initialize_with(
        working_dir,
        |path| async move {
            archon_learning::cozo_guard::open_sqlite_guarded_async(
                &path,
                "open interactive learning db",
            )
            .await
        },
        initialize_schemas,
    )
    .await
}

pub(super) async fn initialize_with<Open, OpenFuture, Initialize>(
    working_dir: &Path,
    open: Open,
    initialize_schemas: Initialize,
) -> InitializedDatabases
where
    Open: FnOnce(String) -> OpenFuture,
    OpenFuture: Future<Output = Result<cozo::DbInstance>>,
    Initialize: FnOnce(&Path, &cozo::DbInstance) -> SchemaInitialization + Send + 'static,
{
    let db_path = crate::command::store_paths::learning_db_path_for_dir(working_dir);
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let db = match open(db_path.to_string_lossy().into_owned()).await {
        Ok(db) => Arc::new(db),
        Err(error) => {
            tracing::warn!(error = %error, "CozoDB learning store unavailable; persistence disabled");
            return InitializedDatabases {
                pipeline: None,
                governed: None,
            };
        }
    };

    let blocking_db = Arc::clone(&db);
    let blocking_working_dir = PathBuf::from(working_dir);
    match archon_tui::observability::spawn_blocking_named(
        "interactive-learning-schema-init",
        move || initialize_schemas(&blocking_working_dir, blocking_db.as_ref()),
    )
    .await
    {
        Ok(result) => InitializedDatabases {
            pipeline: result.pipeline.then(|| Arc::clone(&db)),
            governed: result.governed.then_some(db),
        },
        Err(error) => {
            tracing::warn!(error = %error, "interactive learning schema initialization join failed; persistence disabled");
            InitializedDatabases {
                pipeline: None,
                governed: None,
            }
        }
    }
}

pub(super) fn initialize_schemas(
    working_dir: &Path,
    db: &cozo::DbInstance,
) -> SchemaInitialization {
    let pipeline = match archon_pipeline::learning::schema::initialize_learning_schemas(db) {
        Ok(()) => {
            crate::command::pipeline_learning_migration::maybe_migrate_legacy_pipeline_learning_with_log(
                working_dir,
                &crate::command::store_paths::learning_db_path_for_dir(working_dir),
                db,
                "interactive",
            );
            true
        }
        Err(error) => {
            tracing::warn!(error = %error, "Learning schema init failed; retrain may not work");
            false
        }
    };
    let governed = match archon_learning::schema::ensure_learning_schema(db) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(error = %error, "governed learning schema init failed; runtime evidence disabled");
            false
        }
    };
    SchemaInitialization { pipeline, governed }
}
