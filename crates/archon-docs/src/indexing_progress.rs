use std::time::{Duration, Instant};

use crate::indexing_result::IndexResult;
use crate::models::ChunkArtifact;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndexProgressPhase {
    CandidatesLoaded,
    BatchStarted,
    BatchRetrying,
    BatchStoreStarted,
    BatchStoreFinished,
    ChunkStoreStarted,
    ChunkStoreFinished,
    BatchFinished,
    Complete,
}

#[derive(Clone, Debug)]
pub struct IndexProgress {
    pub phase: IndexProgressPhase,
    pub batch_index: usize,
    pub batch_total: usize,
    pub batch_size: usize,
    pub batch_position: usize,
    pub chunk_id: Option<String>,
    pub indexed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub elapsed: Duration,
}

#[derive(Clone, Copy)]
pub(crate) struct BatchProgress {
    pub(crate) index: usize,
    pub(crate) total: usize,
    pub(crate) started: Instant,
}

pub(crate) fn emit_progress(
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    event: IndexProgress,
) {
    if let Some(callback) = progress.as_deref_mut() {
        callback(event);
    }
}

pub(crate) fn emit_chunk_progress(
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    phase: IndexProgressPhase,
    chunk: &ChunkArtifact,
    batch_size: usize,
    batch_position: usize,
    result: &IndexResult,
    batch: BatchProgress,
) {
    emit_progress(
        progress,
        IndexProgress {
            phase,
            batch_index: batch.index,
            batch_total: batch.total,
            batch_size,
            batch_position,
            chunk_id: Some(chunk.chunk_id.clone()),
            indexed: result.indexed,
            failed: result.failed,
            skipped: result.skipped,
            elapsed: batch.started.elapsed(),
        },
    );
}
