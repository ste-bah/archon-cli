use std::sync::{Arc, Condvar, Mutex};

use crate::embed::LocalEmbeddingProvider;
use crate::errors::DocsError;

#[derive(Debug)]
struct PermitPool {
    available: Mutex<usize>,
    signal: Condvar,
}

impl PermitPool {
    fn new(permits: usize) -> Self {
        Self {
            available: Mutex::new(permits.max(1)),
            signal: Condvar::new(),
        }
    }

    fn acquire(self: &Arc<Self>) -> Permit {
        let mut available = self.available.lock().expect("permit pool lock");
        while *available == 0 {
            available = self.signal.wait(available).expect("permit pool wait");
        }
        *available -= 1;
        Permit {
            pool: Arc::clone(self),
        }
    }
}

struct Permit {
    pool: Arc<PermitPool>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        *self.pool.available.lock().expect("permit pool lock") += 1;
        self.pool.signal.notify_one();
    }
}

struct LimitedProvider {
    inner: Arc<dyn LocalEmbeddingProvider>,
    permits: Arc<PermitPool>,
}

impl LocalEmbeddingProvider for LimitedProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        let _permit = self.permits.acquire();
        self.inner.embed_chunks(chunks)
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        let _permit = self.permits.acquire();
        self.inner.embed_query(query)
    }

    fn dimension(&self) -> usize {
        self.inner.dimension()
    }

    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
}

pub(super) fn consume_embedded_batches(
    provider: Arc<dyn LocalEmbeddingProvider>,
    batches: impl IntoIterator<Item = crate::indexing_parallel::EmbeddingBatch>,
    workers: usize,
    max_in_flight_batches: usize,
    mut consume: impl FnMut(crate::indexing_parallel::EmbeddedBatch, &dyn LocalEmbeddingProvider),
) -> Result<(), DocsError> {
    let worker_count = workers.max(1);
    let max_in_flight = max_in_flight_batches.max(1);
    let provider: Arc<dyn LocalEmbeddingProvider> = Arc::new(LimitedProvider {
        inner: provider,
        permits: Arc::new(PermitPool::new(worker_count)),
    });
    let (job_sender, job_receiver) = std::sync::mpsc::sync_channel(max_in_flight);
    let job_receiver = Arc::new(Mutex::new(job_receiver));
    let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(max_in_flight);
    let worker_result = std::thread::scope(|scope| {
        let handles = spawn_workers(
            scope,
            worker_count,
            &provider,
            &job_receiver,
            &result_sender,
        );
        drop(result_sender);
        schedule_batches(
            job_sender,
            result_receiver,
            batches,
            max_in_flight,
            &provider,
            &mut consume,
        )?;
        join_workers(handles)
    });
    worker_result.map_err(|()| DocsError::Embedding {
        message: "embedding worker panicked".into(),
    })
}

fn spawn_workers<'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    worker_count: usize,
    provider: &Arc<dyn LocalEmbeddingProvider>,
    receiver: &Arc<Mutex<std::sync::mpsc::Receiver<crate::indexing_parallel::EmbeddingBatch>>>,
    sender: &std::sync::mpsc::SyncSender<
        Result<crate::indexing_parallel::EmbeddedBatch, Box<dyn std::any::Any + Send>>,
    >,
) -> Vec<std::thread::ScopedJoinHandle<'scope, ()>> {
    (0..worker_count)
        .map(|_| {
            let provider = Arc::clone(provider);
            let receiver = Arc::clone(receiver);
            let sender = sender.clone();
            scope.spawn(move || worker_loop(provider, receiver, sender))
        })
        .collect()
}

fn worker_loop(
    provider: Arc<dyn LocalEmbeddingProvider>,
    receiver: Arc<Mutex<std::sync::mpsc::Receiver<crate::indexing_parallel::EmbeddingBatch>>>,
    sender: std::sync::mpsc::SyncSender<
        Result<crate::indexing_parallel::EmbeddedBatch, Box<dyn std::any::Any + Send>>,
    >,
) {
    loop {
        let batch = receiver.lock().expect("embedding job receiver lock").recv();
        let Ok(batch) = batch else {
            return;
        };
        let embedded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            embed_one(Arc::clone(&provider), batch)
        }));
        if sender.send(embedded).is_err() {
            return;
        }
    }
}

fn schedule_batches(
    job_sender: std::sync::mpsc::SyncSender<crate::indexing_parallel::EmbeddingBatch>,
    result_receiver: std::sync::mpsc::Receiver<
        Result<crate::indexing_parallel::EmbeddedBatch, Box<dyn std::any::Any + Send>>,
    >,
    batches: impl IntoIterator<Item = crate::indexing_parallel::EmbeddingBatch>,
    max_in_flight: usize,
    provider: &Arc<dyn LocalEmbeddingProvider>,
    consume: &mut impl FnMut(crate::indexing_parallel::EmbeddedBatch, &dyn LocalEmbeddingProvider),
) -> Result<(), ()> {
    let mut batches = batches.into_iter();
    let mut in_flight = submit_initial_batches(&job_sender, &mut batches, max_in_flight)?;
    let mut worker_panicked = false;
    while in_flight > 0 {
        worker_panicked |= consume_one(&result_receiver, provider, consume)?;
        in_flight -= 1;
        if !worker_panicked && let Some(batch) = batches.next() {
            job_sender.send(batch).map_err(|_| ())?;
            in_flight += 1;
        }
    }
    drop(job_sender);
    (!worker_panicked).then_some(()).ok_or(())
}

fn submit_initial_batches(
    sender: &std::sync::mpsc::SyncSender<crate::indexing_parallel::EmbeddingBatch>,
    batches: &mut impl Iterator<Item = crate::indexing_parallel::EmbeddingBatch>,
    max_in_flight: usize,
) -> Result<usize, ()> {
    let mut in_flight = 0;
    for batch in batches.take(max_in_flight) {
        sender.send(batch).map_err(|_| ())?;
        in_flight += 1;
    }
    Ok(in_flight)
}

fn consume_one(
    receiver: &std::sync::mpsc::Receiver<
        Result<crate::indexing_parallel::EmbeddedBatch, Box<dyn std::any::Any + Send>>,
    >,
    provider: &Arc<dyn LocalEmbeddingProvider>,
    consume: &mut impl FnMut(crate::indexing_parallel::EmbeddedBatch, &dyn LocalEmbeddingProvider),
) -> Result<bool, ()> {
    match receiver.recv().map_err(|_| ())? {
        Ok(embedded) => consume(embedded, provider.as_ref()),
        Err(_) => return Ok(true),
    }
    Ok(false)
}

fn join_workers(handles: Vec<std::thread::ScopedJoinHandle<'_, ()>>) -> Result<(), ()> {
    for handle in handles {
        handle.join().map_err(|_| ())?;
    }
    Ok(())
}

fn embed_one(
    provider: Arc<dyn LocalEmbeddingProvider>,
    batch: crate::indexing_parallel::EmbeddingBatch,
) -> crate::indexing_parallel::EmbeddedBatch {
    let texts = batch
        .chunks
        .iter()
        .map(|chunk| chunk.content.clone())
        .collect::<Vec<_>>();
    crate::indexing_parallel::EmbeddedBatch {
        index: batch.index,
        chunks: batch.chunks,
        vectors: provider
            .embed_chunks(&texts)
            .map_err(|error| error.to_string()),
    }
}
