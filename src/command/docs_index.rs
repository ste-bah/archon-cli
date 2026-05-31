use anyhow::Result;
use cozo::DbInstance;
use std::io::{self, Write};
use std::time::Duration;

use archon_docs::indexing::{self, IndexOptions, IndexProgress, IndexProgressPhase, IndexResult};

const DEFAULT_INDEX_WINDOW_SIZE: usize = 1024;

pub(crate) async fn handle_index(
    all: bool,
    document: Option<String>,
    batch_size: usize,
    limit: Option<usize>,
    db: DbInstance,
) -> Result<()> {
    println!("Counting index candidates...");
    flush_stdout();
    let options = IndexOptions {
        all,
        document_id: document,
        batch_size: batch_size.max(1),
        limit,
    };
    let candidates = indexing::count_candidates(&db, &options)
        .map_err(|e| anyhow::anyhow!("count index candidates failed: {e}"))?;
    if candidates == 0 {
        println!("No chunks need indexing.");
        return Ok(());
    }

    print_index_plan(candidates, &options);
    println!("Loading embedding provider...");
    flush_stdout();
    crate::command::docs_embedding::init_embedding(&db)?;
    if archon_docs::embed::get_provider().is_none() {
        let detail = archon_docs::embed::last_init_error()
            .unwrap_or_else(|| "no embedding provider configured".into());
        anyhow::bail!("{detail}. Run 'archon docs model-status' for diagnostics.");
    }
    println!("Embedding provider ready.");
    flush_stdout();
    let result = run_index(&db, &options, candidates)?;

    println!("Indexed: {} chunks", result.indexed);
    if result.failed > 0 {
        println!(
            "Failed:  {} chunks (use 'archon docs model-status' for diagnostics)",
            result.failed
        );
    }
    if result.skipped > 0 {
        println!("Skipped: {} chunks", result.skipped);
    }
    Ok(())
}

fn run_index(db: &DbInstance, options: &IndexOptions, candidates: usize) -> Result<IndexResult> {
    if options.limit.is_none() && !options.all {
        return run_pending_windows(db, options, candidates);
    }
    run_index_pass(db, options)
}

fn run_pending_windows(
    db: &DbInstance,
    options: &IndexOptions,
    candidates: usize,
) -> Result<IndexResult> {
    let mut total = IndexResult::default();
    let mut remaining = candidates;
    let window_size = default_window_size(options.batch_size);
    let mut window_index = 1;
    while remaining > 0 {
        let window_limit = remaining.min(window_size);
        println!(
            "Index window {window_index}: processing up to {window_limit} of {remaining} remaining chunk(s)."
        );
        flush_stdout();
        let pass_options = IndexOptions {
            limit: Some(window_limit),
            ..options.clone()
        };
        let result = run_index_pass(db, &pass_options)?;
        let changed = result.indexed + result.failed + result.skipped;
        total.indexed += result.indexed;
        total.failed += result.failed;
        total.skipped += result.skipped;
        if changed == 0 {
            anyhow::bail!("index window made no progress with {remaining} chunk(s) remaining");
        }
        remaining = indexing::count_candidates(db, options)
            .map_err(|e| anyhow::anyhow!("count index candidates failed after window: {e}"))?;
        println!(
            "Index window {window_index} finished: indexed {}, failed {}, skipped {}, remaining {}.",
            result.indexed, result.failed, result.skipped, remaining
        );
        flush_stdout();
        window_index += 1;
    }
    Ok(total)
}

fn run_index_pass(db: &DbInstance, options: &IndexOptions) -> Result<IndexResult> {
    println!("Loading candidate chunks...");
    flush_stdout();
    indexing::index_chunks_with_progress(db, options, print_progress)
        .map_err(|e| anyhow::anyhow!("index failed: {e}"))
}

fn default_window_size(batch_size: usize) -> usize {
    DEFAULT_INDEX_WINDOW_SIZE.max(batch_size.max(1))
}

fn print_progress(progress: IndexProgress) {
    let elapsed = format_elapsed(progress.elapsed);
    match progress.phase {
        IndexProgressPhase::CandidatesLoaded => {
            println!(
                "Loaded {} candidate chunk(s) in {elapsed}.",
                progress.batch_size
            );
        }
        IndexProgressPhase::BatchStarted => {
            println!(
                "Batch {}/{} started: {} chunk(s), indexed {}, failed {}, elapsed {elapsed}.",
                progress.batch_index,
                progress.batch_total,
                progress.batch_size,
                progress.indexed,
                progress.failed
            );
        }
        IndexProgressPhase::BatchRetrying => {
            println!(
                "Batch {}/{} failed as a batch; retrying {} chunk(s) individually, elapsed {elapsed}.",
                progress.batch_index, progress.batch_total, progress.batch_size
            );
        }
        IndexProgressPhase::BatchStoreStarted => {
            println!(
                "Batch {}/{} bulk store started: {} chunk(s), indexed {}, failed {}, elapsed {elapsed}.",
                progress.batch_index,
                progress.batch_total,
                progress.batch_size,
                progress.indexed,
                progress.failed
            );
        }
        IndexProgressPhase::BatchStoreFinished => {
            println!(
                "Batch {}/{} bulk store finished: indexed {}, failed {}, elapsed {elapsed}.",
                progress.batch_index, progress.batch_total, progress.indexed, progress.failed
            );
        }
        IndexProgressPhase::ChunkStoreStarted => {
            println!(
                "Batch {}/{} chunk {}/{} storing: {}, indexed {}, failed {}, elapsed {elapsed}.",
                progress.batch_index,
                progress.batch_total,
                progress.batch_position,
                progress.batch_size,
                progress.chunk_id.as_deref().unwrap_or("unknown"),
                progress.indexed,
                progress.failed
            );
        }
        IndexProgressPhase::ChunkStoreFinished => {
            println!(
                "Batch {}/{} chunk {}/{} done: indexed {}, failed {}, elapsed {elapsed}.",
                progress.batch_index,
                progress.batch_total,
                progress.batch_position,
                progress.batch_size,
                progress.indexed,
                progress.failed
            );
        }
        IndexProgressPhase::BatchFinished => {
            println!(
                "Batch {}/{} complete: indexed {}, failed {}, skipped {}, elapsed {elapsed}.",
                progress.batch_index,
                progress.batch_total,
                progress.indexed,
                progress.failed,
                progress.skipped
            );
        }
        IndexProgressPhase::Complete => {
            println!(
                "Index pass complete: indexed {}, failed {}, skipped {}, elapsed {elapsed}.",
                progress.indexed, progress.failed, progress.skipped
            );
        }
    }
    flush_stdout();
}

fn format_elapsed(duration: Duration) -> String {
    if duration.as_secs() < 60 {
        format!("{:.1}s", duration.as_secs_f64())
    } else {
        let minutes = duration.as_secs() / 60;
        let seconds = duration.as_secs() % 60;
        format!("{minutes}m{seconds:02}s")
    }
}

fn flush_stdout() {
    let _ = io::stdout().flush();
}

fn print_index_plan(candidates: usize, options: &IndexOptions) {
    let scope = options
        .document_id
        .as_deref()
        .map(|id| format!("document {id}"))
        .unwrap_or_else(|| "all documents".into());
    let mode = if options.all {
        "all chunks"
    } else {
        "pending chunks"
    };
    let limit = options
        .limit
        .map(|limit| format!(", limit {limit}"))
        .unwrap_or_default();
    println!(
        "Indexing {candidates} candidate chunk(s): {mode}, {scope}, batch {}{limit}",
        options.batch_size
    );
}
