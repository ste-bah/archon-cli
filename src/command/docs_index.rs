use anyhow::Result;
use cozo::DbInstance;
use std::io::{self, Write};
use std::time::Duration;

use archon_docs::indexing::{self, IndexOptions, IndexProgress, IndexProgressPhase, IndexResult};
use archon_docs::{embed, index_jobs, index_queue};

const DEFAULT_INDEX_WINDOW_SIZE: usize = 1024;
const INDEX_LEASE_SECS: u64 = 900;

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
        ..Default::default()
    };
    let candidates = prepare_candidates(&db, &options)?;
    if candidates == 0 {
        println!("No chunks need indexing.");
        return Ok(());
    }

    let _writer_lock = crate::command::docs_index_lock::DocsIndexLock::acquire()?;
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
    let summary = provider_summary();
    print_index_tuning(&summary, &options);
    let job_id = index_jobs::start_job(
        &db,
        index_scope(&options),
        options.document_id.as_deref(),
        &summary.name,
        summary.dimension,
    )?;
    let result = match run_index(&db, &options, candidates, &job_id) {
        Ok(result) => {
            index_jobs::finish_job(&db, &job_id, None)?;
            result
        }
        Err(error) => {
            let _ = index_jobs::finish_job(&db, &job_id, Some(&error.to_string()));
            return Err(error);
        }
    };

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
    if result.cache_hits > 0 {
        println!("Cache hits: {} chunks", result.cache_hits);
    }
    Ok(())
}

pub(crate) fn handle_index_status(db: DbInstance) -> Result<()> {
    let stats = index_queue::stats(&db)?;
    let jobs = index_jobs::summary(&db)?;
    println!("Index queue:");
    println!("  Pending: {}", stats.pending);
    println!("  Leased:  {}", stats.leased);
    println!("  Indexed: {}", stats.indexed);
    println!("  Failed:  {}", stats.failed);
    println!("Index jobs:");
    println!("  Running:   {}", jobs.running);
    println!("  Paused:    {}", jobs.paused);
    println!("  Completed: {}", jobs.completed);
    println!("  Failed:    {}", jobs.failed);
    println!("  Cancelled: {}", jobs.cancelled);
    let recent_jobs = index_jobs::list_recent(&db, 5)?;
    if !recent_jobs.is_empty() {
        println!("Recent index jobs:");
        for job in recent_jobs {
            println!(
                "  {} status={} leased={} indexed={} failed={} scope={} started={}",
                job.job_id,
                job.status,
                job.leased,
                job.indexed,
                job.failed,
                job.scope,
                job.started_at
            );
        }
    }
    let failures = index_queue::failed_rows(&db, 5)?;
    if !failures.is_empty() {
        println!("Recent failed queue rows:");
        for failure in failures {
            println!(
                "  {} attempts={} error={}",
                failure.chunk_id, failure.attempt_count, failure.last_error
            );
        }
    }
    Ok(())
}

pub(crate) fn handle_index_retry_failed(db: DbInstance, limit: Option<usize>) -> Result<()> {
    let retried = index_queue::retry_failed(&db, limit)?;
    println!("Requeued {retried} failed chunk(s) for indexing.");
    Ok(())
}

pub(crate) fn handle_index_pause(db: DbInstance, job_id: &str) -> Result<()> {
    index_jobs::pause_job(&db, job_id)?;
    let released = index_queue::release_leases_for_owner(&db, job_id)?;
    println!("Paused index job {job_id}. Released {released} leased chunk(s) back to pending.");
    Ok(())
}

pub(crate) fn handle_index_resume(db: DbInstance, job_id: &str) -> Result<()> {
    index_jobs::resume_job(&db, job_id)?;
    println!("Marked index job {job_id} resumable. Run 'archon docs index' to drain queued work.");
    Ok(())
}

pub(crate) fn handle_index_cancel(db: DbInstance, job_id: &str) -> Result<()> {
    index_jobs::cancel_job(&db, job_id)?;
    let released = index_queue::release_leases_for_owner(&db, job_id)?;
    println!("Cancelled index job {job_id}. Released {released} leased chunk(s) back to pending.");
    Ok(())
}

fn prepare_candidates(db: &DbInstance, options: &IndexOptions) -> Result<usize> {
    if options.all {
        return indexing::count_candidates(db, options)
            .map_err(|e| anyhow::anyhow!("count index candidates failed: {e}"));
    }
    let queued = index_queue::count_pending(db, options.document_id.as_deref())
        .map_err(|e| anyhow::anyhow!("count queued index candidates failed: {e}"))?;
    if queued > 0 {
        return Ok(options
            .limit
            .map(|limit| queued.min(limit))
            .unwrap_or(queued));
    }
    let enqueued =
        index_queue::backfill_pending_chunks(db, options.document_id.as_deref(), options.limit)
            .map_err(|e| anyhow::anyhow!("enqueue pending chunks failed: {e}"))?;
    if enqueued > 0 {
        println!("Queued {enqueued} pending chunk(s) for indexing.");
    }
    let pending = index_queue::count_pending(db, options.document_id.as_deref())
        .map_err(|e| anyhow::anyhow!("count queued index candidates failed: {e}"))?;
    Ok(options
        .limit
        .map(|limit| pending.min(limit))
        .unwrap_or(pending))
}

fn run_index(
    db: &DbInstance,
    options: &IndexOptions,
    candidates: usize,
    job_id: &str,
) -> Result<IndexResult> {
    if !options.all {
        return run_queue_windows(db, options, candidates, job_id);
    }
    let result = run_index_pass(db, options)?;
    index_jobs::record_progress(
        db,
        job_id,
        result.indexed + result.failed + result.skipped,
        result.indexed,
        result.failed,
        result.skipped,
    )?;
    Ok(result)
}

fn run_queue_windows(
    db: &DbInstance,
    options: &IndexOptions,
    candidates: usize,
    job_id: &str,
) -> Result<IndexResult> {
    let mut total = IndexResult::default();
    let mut remaining = candidates;
    let window_size = default_window_size(options.batch_size);
    let mut window_index = 1;
    let owner = job_id.to_string();
    while remaining > 0 {
        if let Some(status) = index_jobs::control_status(db, job_id)? {
            println!("Index job {job_id} is {status}; leaving remaining queue work for later.");
            break;
        }
        let window_limit = remaining.min(window_size);
        println!(
            "Index window {window_index}: processing up to {window_limit} of {remaining} remaining chunk(s)."
        );
        flush_stdout();
        let chunks = index_queue::lease_pending_chunks(
            db,
            &owner,
            window_limit,
            INDEX_LEASE_SECS,
            options.document_id.as_deref(),
        )
        .map_err(|e| anyhow::anyhow!("lease queued chunks failed: {e}"))?;
        if chunks.is_empty() {
            anyhow::bail!("index queue returned no chunks with {remaining} expected");
        }
        let result = run_loaded_index_pass(db, chunks, options)?;
        let changed = result.indexed + result.failed + result.skipped;
        index_jobs::record_progress(
            db,
            job_id,
            changed,
            result.indexed,
            result.failed,
            result.skipped,
        )?;
        total.indexed += result.indexed;
        total.failed += result.failed;
        total.skipped += result.skipped;
        total.cache_hits += result.cache_hits;
        if changed == 0 {
            anyhow::bail!("index window made no progress with {remaining} chunk(s) remaining");
        }
        remaining = match options.limit {
            Some(_) => remaining.saturating_sub(changed),
            None => index_queue::count_pending(db, options.document_id.as_deref())
                .map_err(|e| anyhow::anyhow!("count queued candidates failed after window: {e}"))?,
        };
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

fn run_loaded_index_pass(
    db: &DbInstance,
    chunks: Vec<archon_docs::models::ChunkArtifact>,
    options: &IndexOptions,
) -> Result<IndexResult> {
    println!("Loaded {} leased chunk(s).", chunks.len());
    flush_stdout();
    indexing::index_loaded_chunks_with_options_progress(db, chunks, options, print_progress)
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

fn print_index_tuning(provider: &ProviderSummary, options: &IndexOptions) {
    let requested_workers = options.effective_embedding_workers(&provider.name);
    let workers = requested_workers.min(provider.max_workers.max(1)).max(1);
    let in_flight = options.effective_max_in_flight_batches(workers);
    let writer_batch = options.effective_writer_batch_size();
    let capped = if requested_workers > workers {
        format!(", requested_workers {requested_workers} capped by provider")
    } else {
        String::new()
    };
    println!(
        "Index tuning: embedding_workers {workers}, max_in_flight_batches {in_flight}, writer_batch_size {writer_batch}, single_writer true{capped}"
    );
}

struct ProviderSummary {
    name: String,
    dimension: usize,
    max_workers: usize,
}

fn provider_summary() -> ProviderSummary {
    embed::get_provider()
        .map(|provider| ProviderSummary {
            name: provider.backend_name().to_string(),
            dimension: provider.dimension(),
            max_workers: provider.max_embedding_workers(),
        })
        .unwrap_or_else(|| ProviderSummary {
            name: "unknown".into(),
            dimension: 0,
            max_workers: 1,
        })
}

fn index_scope(options: &IndexOptions) -> &str {
    if options.all {
        "all"
    } else if options.document_id.is_some() {
        "document"
    } else {
        "pending"
    }
}
