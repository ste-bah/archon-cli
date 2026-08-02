use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use cozo::{DataValue, ScriptMutability};

use super::{CozoGuardConfig, open_sqlite_guarded_instance};

const WRITERS: usize = 8;
const OPS_PER_WRITER: usize = 16;

#[test]
fn sqlite_guarded_writers_persist_exact_bounded_row_count_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("throughput.db");
    let config = CozoGuardConfig::for_db_path(&path);
    create_relation(&path, config.clone());
    let started = Instant::now();
    let errors = run_writers(path.clone(), config.clone());
    let elapsed = started.elapsed();
    assert!(errors.is_empty(), "guarded writer errors: {errors:?}");
    let rows = count_rows(&path, config);
    assert_eq!(rows, WRITERS * OPS_PER_WRITER);
    println!(
        "EVIDENCE cozo_sqlite_guarded_throughput writers={WRITERS} ops={} rows={rows} elapsed_ms={} ops_per_sec={:.2} errors={}",
        WRITERS * OPS_PER_WRITER,
        elapsed.as_millis(),
        rows as f64 / elapsed.as_secs_f64(),
        errors.len(),
    );
}

fn create_relation(path: &std::path::Path, config: CozoGuardConfig) {
    open_sqlite_guarded_instance(path.to_str().unwrap(), "create throughput database", config)
        .unwrap()
        .run_script_guarded(
            ":create throughput_rows { id: Int => writer: Int }",
            BTreeMap::new(),
            ScriptMutability::Mutable,
            "create throughput relation",
        )
        .unwrap();
}

fn run_writers(path: PathBuf, config: CozoGuardConfig) -> Vec<anyhow::Error> {
    let barrier = Arc::new(std::sync::Barrier::new(WRITERS + 1));
    let handles = (0..WRITERS)
        .map(|writer| spawn_writer(path.clone(), config.clone(), Arc::clone(&barrier), writer))
        .collect::<Vec<_>>();
    barrier.wait();
    handles
        .into_iter()
        .filter_map(|handle| handle.join().unwrap().err())
        .collect()
}

fn spawn_writer(
    path: PathBuf,
    config: CozoGuardConfig,
    barrier: Arc<std::sync::Barrier>,
    writer: usize,
) -> std::thread::JoinHandle<anyhow::Result<()>> {
    std::thread::spawn(move || {
        let database =
            open_sqlite_guarded_instance(path.to_str().unwrap(), "open throughput writer", config)?;
        barrier.wait();
        for operation in 0..OPS_PER_WRITER {
            database.run_script_guarded(
                "?[id, writer] <- [[$id, $writer]] :put throughput_rows { id => writer }",
                writer_params(writer, operation),
                ScriptMutability::Mutable,
                "throughput write",
            )?;
        }
        Ok(())
    })
}

fn writer_params(writer: usize, operation: usize) -> BTreeMap<String, DataValue> {
    BTreeMap::from([
        (
            "id".into(),
            DataValue::from((writer * OPS_PER_WRITER + operation) as i64),
        ),
        ("writer".into(), DataValue::from(writer as i64)),
    ])
}

fn count_rows(path: &std::path::Path, config: CozoGuardConfig) -> usize {
    open_sqlite_guarded_instance(path.to_str().unwrap(), "reopen throughput database", config)
        .unwrap()
        .run_script_guarded(
            "?[count(id)] := *throughput_rows{id}",
            BTreeMap::new(),
            ScriptMutability::Immutable,
            "count throughput rows",
        )
        .unwrap()
        .rows[0][0]
        .get_int()
        .unwrap() as usize
}
