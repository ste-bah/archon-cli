//! Two-process contention harness for issues #140 and #144.
//!
//! Not a unit test on purpose: both bugs are cross-*process* races on one
//! `.archon/leann.db`, and the write lock they contend for is an OS byte-range
//! lock. A single test binary cannot produce that -- `run_guarded_once` treats a
//! lock this thread already holds as re-entrant, so two threads never actually
//! race the file lock.
//!
//! Subcommands:
//!   seed   <dir> <count>            write a synthetic Rust corpus
//!   schema <db> <label> [start_ms]  open the index (this runs `ensure_schema`)
//!   index  <db> <root> <label> [start_ms]
//!                                   full repository index pass
//!   count  <db>                     rows in `file_states` and `code_chunks`
//!
//! `start_ms` is a unix-millis rendezvous: every process sleeps until it, so
//! the two really do enter the contended region together rather than one
//! finishing while the other is still linking.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use archon_leann::indexer::{EmbeddingConfig, EmbeddingProviderKind};
use archon_leann::{CodeIndex, IndexConfig};

/// Mock embeddings, small vectors.
///
/// The bugs live in the persistence path, not the embedder, and a real provider
/// would dominate the run with model load time while *reducing* contention --
/// slow embedding means the write lock is idle most of the wall clock. Zero
/// vectors keep the writes back-to-back, which is the pressure that reproduces.
const DIMENSION: usize = 32;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,archon_leann=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("");
    match command {
        "seed" => seed(
            Path::new(&args[2]),
            args[3].parse().expect("count"),
            args.get(4)
                .and_then(|value| value.parse().ok())
                .unwrap_or(8),
        ),
        "schema" => schema(&args[2], &args[3], args.get(4)),
        "schema-nolock" => schema_nolock(&args[2], &args[3], args.get(4)),
        "index" => index(&args[2], &args[3], &args[4], args.get(5), true),
        "index-failfast" => index(&args[2], &args[3], &args[4], args.get(5), false),
        "count" => count(&args[2]),
        "hold" => hold(&args[2], args[3].parse().expect("seconds")),
        other => {
            eprintln!("unknown subcommand {other:?}");
            std::process::exit(2);
        }
    }
}

fn wait_for_start(start_ms: Option<&String>) {
    let Some(start_ms) = start_ms.and_then(|value| value.parse::<u128>().ok()) else {
        return;
    };
    loop {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        if now >= start_ms {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Many small files rather than a few large ones.
///
/// The contended resource is the write lock, and it is taken once per *file*.
/// Chunk count only buys HNSW insert time, which slows the run down without
/// making the race any likelier.
fn seed(dir: &Path, count: usize, functions: usize) {
    std::fs::create_dir_all(dir).expect("create corpus dir");
    for file in 0..count {
        let body = (0..functions)
            .map(|item| {
                format!(
                    "pub fn generated_{file}_{item}(input: u64) -> u64 {{\n    \
                     let scratch = input.wrapping_mul({item}).wrapping_add({file});\n    \
                     scratch ^ (scratch >> 7)\n}}\n"
                )
            })
            .collect::<String>();
        std::fs::write(dir.join(format!("generated_{file}.rs")), body).expect("write corpus file");
    }
    println!("seeded {count} files into {}", dir.display());
}

fn open_index(db: &str) -> anyhow::Result<CodeIndex> {
    CodeIndex::new(
        db,
        EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: DIMENSION,
        },
    )
}

fn schema(db: &str, label: &str, start_ms: Option<&String>) {
    wait_for_start(start_ms);
    let started = std::time::Instant::now();
    match open_index(db) {
        Ok(_) => println!(
            "{label}: SCHEMA OK in {:.1}s",
            started.elapsed().as_secs_f64()
        ),
        Err(error) => {
            println!(
                "{label}: SCHEMA FAILED after {:.1}s: {error:#}",
                started.elapsed().as_secs_f64()
            );
            std::process::exit(1);
        }
    }
}

/// `ensure_schema` with the cross-process write lock deliberately absent.
///
/// `CodeIndex::from_db` builds an `Indexer` on the default guard config, which
/// has no `write_lock_path`, so exclusion degrades to a per-process mutex. This
/// is the only way two processes get inside `:create` at the same time, and it
/// is what shows whether `run_idempotent`'s benign match covers the racing
/// error shape as well as the sequential one.
fn schema_nolock(db: &str, label: &str, start_ms: Option<&String>) {
    let handle = cozo::DbInstance::new("sqlite", db, "").expect("open sqlite");
    wait_for_start(start_ms);
    let started = std::time::Instant::now();
    let config = EmbeddingConfig {
        provider: EmbeddingProviderKind::Mock,
        dimension: DIMENSION,
    };
    match CodeIndex::from_db(handle, config) {
        Ok(_) => println!(
            "{label}: SCHEMA OK in {:.1}s",
            started.elapsed().as_secs_f64()
        ),
        Err(error) => {
            println!(
                "{label}: SCHEMA FAILED after {:.1}s: {error:#}",
                started.elapsed().as_secs_f64()
            );
            std::process::exit(1);
        }
    }
}

/// One repository pass. `queue` picks the write-lock acquire strategy.
///
/// `queue = false` reproduces the pre-fix acquire -- sample the lock once per
/// retry attempt, sleeping 100ms rising to 2s in between -- while leaving the
/// skip-on-contention net in place. That isolates the two halves of the #140
/// fix: without the net this arm *fails*, and with the net but without queueing
/// it survives while skipping most of its work. Only both together produce a
/// pass that is complete as well as alive.
fn index(db: &str, root: &str, label: &str, start_ms: Option<&String>, queue: bool) {
    let mut guard = archon_cozo::CozoGuardConfig::for_db_path(db);
    if queue {
        guard = guard.with_write_lock_wait(archon_cozo::DEFAULT_WRITE_LOCK_WAIT);
    }
    let handle = match archon_cozo::open_sqlite_guarded(db, "harness open", &guard) {
        Ok(handle) => handle,
        Err(error) => {
            println!("{label}: OPEN FAILED: {error:#}");
            std::process::exit(1);
        }
    };
    let embedding = EmbeddingConfig {
        provider: EmbeddingProviderKind::Mock,
        dimension: DIMENSION,
    };
    let index = archon_leann::indexer::Indexer::with_guard(handle, guard, embedding, None)
        .expect("build indexer");
    index.ensure_schema().expect("schema");
    let config = IndexConfig {
        root_path: Path::new(root).to_path_buf(),
        include_patterns: Vec::new(),
        exclude_patterns: Vec::new(),
    };
    wait_for_start(start_ms);
    let started = std::time::Instant::now();
    let outcome = index.index_repository_blocking_with_cancel(
        Path::new(root),
        &config,
        &std::sync::atomic::AtomicBool::new(false),
    );
    let elapsed = started.elapsed().as_secs_f64();
    match outcome {
        Ok(stats) => println!(
            "{label}: INDEX OK in {elapsed:.1}s files={} chunks={} skipped={} db_bytes={}",
            stats.total_files,
            stats.total_chunks,
            stats.skipped_files,
            db_bytes(db)
        ),
        Err(error) => {
            println!(
                "{label}: INDEX FAILED after {elapsed:.1}s db_bytes={}: {error:#}",
                db_bytes(db)
            );
            std::process::exit(1);
        }
    }
}

/// Hold the store's write lock for `seconds`, doing nothing else.
///
/// A stand-in for a peer that is mid-persist for longer than the indexer is
/// willing to wait. Two real indexers cannot demonstrate the resume property on
/// their own: whichever file one of them skips, the other one usually indexes,
/// so `file_states` ends up complete either way and nothing is proved. A holder
/// that indexes nothing leaves the skipped files genuinely unindexed, which is
/// what makes the follow-up run's behaviour meaningful.
fn hold(db: &str, seconds: u64) {
    let lock_path = archon_cozo::write_lock_path_for_db(db);
    println!("holder: taking {} for {seconds}s", lock_path.display());
    archon_cozo::with_write_lock_blocking(&lock_path, "harness holder", || {
        std::thread::sleep(Duration::from_secs(seconds));
        Ok(())
    })
    .expect("hold write lock");
    println!("holder: released");
}

fn db_bytes(db: &str) -> u64 {
    std::fs::metadata(db).map(|meta| meta.len()).unwrap_or(0)
}

fn count(db: &str) {
    let guard = archon_cozo::CozoGuardConfig::for_db_path(db);
    let handle = archon_cozo::open_sqlite_guarded(db, "harness count", &guard).expect("open db");
    for relation in ["file_states", "code_chunks"] {
        let rows = archon_cozo::run_script_guarded(
            &handle,
            &format!("?[count(file_path)] := *{relation}{{file_path}}"),
            BTreeMap::new(),
            cozo::ScriptMutability::Immutable,
            "harness count",
            &guard,
        )
        .expect("count query");
        let value = rows
            .rows
            .first()
            .and_then(|row| row.first())
            .map(|value| format!("{value:?}"))
            .unwrap_or_else(|| "0".to_string());
        println!("{relation}={value}");
    }
    println!("db_bytes={}", db_bytes(db));
}
