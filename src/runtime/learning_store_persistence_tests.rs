use std::sync::{Arc, Barrier};

use anyhow::Result;
use cozo::{DbInstance, ScriptMutability};

use super::{acquire_for_path, clear_for_tests};

const ACQUIRERS: usize = 8;
const RELATION: &str = "provider_runtime_events";

#[test]
fn concurrent_runtime_acquisition_persists_governed_learning_row_after_reopen() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("learning-state.db");
    clear_for_tests(&path);
    let handles = concurrent_acquires(path.clone());
    assert_one_arc(&handles);
    let original = Arc::clone(&handles[0]);
    persist_governed_row(&handles[0])?;
    drop(handles);
    clear_for_tests(&path);
    let reopened = acquire_for_path(&path)?;
    assert!(!Arc::ptr_eq(&original, &reopened));
    let relation_count = relation_count(&reopened, RELATION)?;
    let row_count = runtime_event_count(&reopened)?;
    assert_eq!(relation_count, 1);
    assert_eq!(row_count, 1);
    println!(
        "EVIDENCE runtime_learning_store_acquisition acquirers={ACQUIRERS} canonical_arc=true relations={relation_count} rows={row_count}"
    );
    drop(original);
    drop(reopened);
    clear_for_tests(&path);
    Ok(())
}

fn concurrent_acquires(path: std::path::PathBuf) -> Vec<Arc<DbInstance>> {
    let barrier = Arc::new(Barrier::new(ACQUIRERS + 1));
    let handles = (0..ACQUIRERS)
        .map(|_| spawn_acquire(path.clone(), Arc::clone(&barrier)))
        .collect::<Vec<_>>();
    barrier.wait();
    handles
        .into_iter()
        .map(|handle| handle.join().expect("acquisition thread panicked").unwrap())
        .collect()
}

fn spawn_acquire(
    path: std::path::PathBuf,
    barrier: Arc<Barrier>,
) -> std::thread::JoinHandle<Result<Arc<DbInstance>>> {
    std::thread::spawn(move || {
        barrier.wait();
        acquire_for_path(&path)
    })
}

fn assert_one_arc(databases: &[Arc<DbInstance>]) {
    assert!(
        databases
            .windows(2)
            .all(|pair| Arc::ptr_eq(&pair[0], &pair[1]))
    );
}

fn persist_governed_row(database: &DbInstance) -> Result<()> {
    database.run_script(
        "?[event_id, provider_id, runtime_mode, event_type, severity, created_at] <- [[\"evidence-event\", \"test\", \"test\", \"evidence\", \"info\", \"0\"]] :put provider_runtime_events { event_id => provider_id, runtime_mode, event_type, severity, created_at }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(())
}

fn runtime_event_count(database: &DbInstance) -> Result<i64> {
    Ok(database
        .run_script(
            "?[count(event_id)] := *provider_runtime_events{event_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .rows[0][0]
        .get_int()
        .unwrap())
}

fn relation_count(database: &DbInstance, relation: &str) -> Result<usize> {
    let rows = database
        .run_script(
            "::relations",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let name_column = rows
        .headers
        .iter()
        .position(|header| header == "name")
        .unwrap();
    Ok(rows
        .rows
        .iter()
        .filter(|row| row[name_column].get_str() == Some(relation))
        .count())
}
