//! A reserved SQLite writer allows metadata reads, then rejects Cozo's mutation.
use std::sync::mpsc;
use std::time::Duration;

use crate::{CozoGuardConfig, render_cozo_error};
use cozo::{DbInstance, ScriptMutability};

fn mutation_after_metadata_read(
    script: &'static str,
    asynchronous: bool,
    retries: usize,
    expected: [i64; 2],
) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mutation.db");
    let db = DbInstance::new("sqlite", path.to_str().unwrap(), "").unwrap();
    db.run_script(
        ":create values { key: Int => value: Int }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        "?[key, value] <- [[1, 1]] :put values {key => value}",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    let (ready, started) = mpsc::channel();
    let (commit, requested) = mpsc::channel();
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        let connection = sqlite::Connection::open(writer_path).unwrap();
        connection
            .execute("BEGIN IMMEDIATE; UPDATE cozo SET v = v;")
            .unwrap();
        ready.send(()).unwrap();
        let _ = requested.recv();
        connection.execute("COMMIT;").unwrap();
    });
    started.recv_timeout(Duration::from_secs(10)).unwrap();
    // Prove this is the reviewer's mutation failure, not a busy metadata read.
    db.run_script(
        "?[key, value] := *values {key, value}",
        Default::default(),
        ScriptMutability::Immutable,
    )
    .unwrap();
    let raw = db
        .run_script(script, Default::default(), ScriptMutability::Mutable)
        .unwrap_err();
    assert!(
        raw.to_string().contains("when executing against relation"),
        "{raw}"
    );
    assert!(
        render_cozo_error(&raw).contains("locked (code 5)"),
        "{}",
        render_cozo_error(&raw)
    );
    let release = commit.clone();
    let (observed, evidence) = mpsc::channel();
    let mut attempts = 0;
    let result = crate::with_busy_observer(
        move |_, error| {
            assert!(error.contains("when executing against relation"), "{error}");
            assert!(error.contains("locked (code 5)"), "{error}");
            attempts += 1;
            observed.send(()).unwrap();
            if attempts == retries {
                release.send(()).unwrap();
            }
        },
        || {
            let config = CozoGuardConfig {
                initial_backoff: Duration::from_millis(1),
                max_backoff: Duration::from_millis(5),
                ..Default::default()
            };
            if asynchronous {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(crate::run_script_guarded_async(
                        std::sync::Arc::new(db),
                        script,
                        Default::default(),
                        ScriptMutability::Mutable,
                        "contended mutation",
                        &config,
                    ))
            } else {
                crate::run_script_guarded(
                    &db,
                    script,
                    Default::default(),
                    ScriptMutability::Mutable,
                    "contended mutation",
                    &config,
                )
            }
        },
    );
    // Always release the real writer, including on the old terminal-error path.
    let _ = commit.send(());
    writer.join().unwrap();
    assert!(
        evidence.try_iter().count() >= retries,
        "mutation cause was discarded: {result:?}"
    );
    result.unwrap();
    let reopened = DbInstance::new("sqlite", path.to_str().unwrap(), "").unwrap();
    let rows = reopened
        .run_script(
            "?[key, value] := *values {key, value}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert!(
        rows.rows
            .contains(&expected.into_iter().map(cozo::DataValue::from).collect()),
        "mutation was not persisted: {rows:?}"
    );
}

#[test]
fn reserved_writer_retries_a_real_put() {
    mutation_after_metadata_read(
        "?[key, value] <- [[2, 2]] :put values {key => value}",
        false,
        1,
        [2, 2],
    );
}
#[test]
fn reserved_writer_retries_a_real_update_repeatedly() {
    mutation_after_metadata_read(
        "?[key, value] <- [[1, 2]] :update values {key => value}",
        false,
        3,
        [1, 2],
    );
}
#[test]
fn reserved_writer_retries_a_real_async_put() {
    mutation_after_metadata_read(
        "?[key, value] <- [[3, 3]] :put values {key => value}",
        true,
        2,
        [3, 3],
    );
}
