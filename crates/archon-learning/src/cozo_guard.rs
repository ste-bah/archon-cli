use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, NamedRows, ScriptMutability};

pub(crate) fn run_script_guarded(
    db: &DbInstance,
    script: &str,
    params: BTreeMap<String, DataValue>,
    mutability: ScriptMutability,
    context: &str,
) -> Result<NamedRows> {
    archon_cozo::run_bound_script_guarded(db, script, params, mutability, context)
}

pub fn open_sqlite_guarded(path: &str, context: &str) -> Result<std::sync::Arc<DbInstance>> {
    let config = archon_cozo::CozoGuardConfig::for_db_path(path);
    archon_cozo::open_sqlite_guarded_instance(path, context, config)
        .map(|database| database.db_arc())
}

pub async fn open_sqlite_guarded_async(
    path: &str,
    context: &str,
) -> Result<std::sync::Arc<DbInstance>> {
    let config = archon_cozo::CozoGuardConfig::for_interactive_db_path(path);
    let db = archon_cozo::open_sqlite_guarded_async(path, context, &config).await?;
    Ok(archon_cozo::GuardedDbInstance::new(db, config).db_arc())
}

#[cfg(test)]
pub(crate) fn test_sqlite_db(prefix: &str) -> std::sync::Arc<DbInstance> {
    let path = format!("/tmp/{prefix}-{}.db", uuid::Uuid::new_v4());
    let db = open_sqlite_guarded(&path, "open test learning store").unwrap();
    crate::schema::ensure_learning_schema(&db).unwrap();
    db
}

#[cfg(test)]
mod tests {
    #[test]
    fn mutable_write_without_registered_guard_config_fails_loud() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("unregistered.db");
        let database = cozo::DbInstance::new("sqlite", &path, "").unwrap();

        let error = super::run_script_guarded(
            &database,
            ":create unregistered_write { id: String => value: String }",
            Default::default(),
            cozo::ScriptMutability::Mutable,
            "unregistered learning write",
        )
        .expect_err("unregistered mutable write must fail");

        assert!(
            error.to_string().contains("no bound Cozo guard config"),
            "unexpected error: {error:#}",
        );
    }

    #[test]
    fn retryable_errors_include_sqlite_busy_messages() {
        assert!(archon_cozo::is_retryable_cozo_error(
            "database is locked (code 5)"
        ));
        assert!(archon_cozo::is_retryable_cozo_error(
            "Error { code: Some(5) }"
        ));
        assert!(!archon_cozo::is_retryable_cozo_error("relation not found"));
    }

    #[cfg(unix)]
    #[test]
    fn public_write_path_uses_registered_final_file_alias_lock() {
        use std::os::unix::fs::symlink;
        use std::sync::mpsc;
        use std::time::Duration;

        let temp = tempfile::tempdir().unwrap();
        let real_path = temp.path().join("learning.db");
        let alias_path = temp.path().join("learning-alias.db");
        let database =
            super::open_sqlite_guarded(real_path.to_str().unwrap(), "open test learning store")
                .unwrap();
        crate::schema::ensure_learning_schema(&database).unwrap();
        symlink(&real_path, &alias_path).unwrap();
        let alias_lock = archon_cozo::write_lock_path_for_db(&alias_path);
        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let holder = std::thread::spawn(move || {
            archon_cozo::with_write_lock(&alias_lock, "hold alias lock", || {
                locked_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
        });
        locked_rx.recv_timeout(Duration::from_secs(2)).unwrap();

        let event = crate::models::LearningEvent {
            event_id: "event-1".into(),
            workspace_id: "workspace-1".into(),
            event_type: crate::models::LearningEventType::TestPassed,
            source_artifact_id: String::new(),
            outcome_artifact_id: None,
            signal: serde_json::json!({}),
            confidence: 1.0,
            provenance_record_id: String::new(),
            created_at: "2026-07-20T00:00:00Z".into(),
        };
        let (write_tx, write_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            write_tx
                .send(crate::store::insert_learning_event(&database, &event))
                .unwrap();
        });

        assert!(
            write_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "public learning write bypassed registered alias lock",
        );
        release_tx.send(()).unwrap();
        holder.join().unwrap().unwrap();
        write_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        writer.join().unwrap();
    }

    #[test]
    fn public_writes_to_distinct_databases_do_not_share_a_lock() {
        use std::sync::mpsc;
        use std::time::Duration;

        let temp = tempfile::tempdir().unwrap();
        let first_path = temp.path().join("first.db");
        let second_path = temp.path().join("second.db");
        let first =
            super::open_sqlite_guarded(first_path.to_str().unwrap(), "open first learning store")
                .unwrap();
        let second =
            super::open_sqlite_guarded(second_path.to_str().unwrap(), "open second learning store")
                .unwrap();
        crate::schema::ensure_learning_schema(&first).unwrap();
        crate::schema::ensure_learning_schema(&second).unwrap();
        let first_lock = archon_cozo::write_lock_path_for_db(&first_path);
        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let holder = std::thread::spawn(move || {
            archon_cozo::with_write_lock(&first_lock, "hold first lock", || {
                locked_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
        });
        locked_rx.recv_timeout(Duration::from_secs(2)).unwrap();

        let event = crate::models::LearningEvent {
            event_id: "event-2".into(),
            workspace_id: "workspace-2".into(),
            event_type: crate::models::LearningEventType::TestPassed,
            source_artifact_id: String::new(),
            outcome_artifact_id: None,
            signal: serde_json::json!({}),
            confidence: 1.0,
            provenance_record_id: String::new(),
            created_at: "2026-07-20T00:00:00Z".into(),
        };
        let (write_tx, write_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            write_tx
                .send(crate::store::insert_learning_event(&second, &event))
                .unwrap();
        });

        write_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second database write waited on first database lock")
            .unwrap();
        release_tx.send(()).unwrap();
        holder.join().unwrap().unwrap();
        writer.join().unwrap();
    }

    #[test]
    fn sqlite_database_retains_its_write_lock_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("learning.db");
        let database =
            super::open_sqlite_guarded(path.to_str().unwrap(), "open test learning store").unwrap();

        assert_eq!(
            archon_cozo::guarded_config_for(&database).and_then(|config| config.write_lock_path),
            Some(archon_cozo::write_lock_path_for_db(&path)),
        );
    }
}
