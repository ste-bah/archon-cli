use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use super::*;
use crate::errors::DocsError;

struct MockProvider;

impl LocalEmbeddingProvider for MockProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        Ok(chunks
            .iter()
            .map(|chunk| vec![chunk.len() as f32, 1.0])
            .collect())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        Ok(vec![query.len() as f32, 1.0])
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "mock"
    }
}

fn chunk(id: &str, content: &str) -> ChunkArtifact {
    ChunkArtifact {
        chunk_id: id.into(),
        document_id: "doc-a".into(),
        artifact_id: "artifact-a".into(),
        chunk_index: 1,
        page_start: 1,
        page_end: 1,
        content: content.into(),
        content_hash: format!("hash-{id}"),
        embedding_status: "pending".into(),
    }
}

struct CoordinatedProvider {
    second_started: Arc<(Mutex<bool>, Condvar)>,
    release_second: Arc<(Mutex<bool>, Condvar)>,
}

impl LocalEmbeddingProvider for CoordinatedProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        if chunks[0] == "first" {
            let (started, signal) = &*self.second_started;
            let started = started.lock().expect("second-started lock");
            let (started, timeout) = signal
                .wait_timeout_while(started, Duration::from_secs(2), |started| !*started)
                .expect("second-started wait");
            assert!(!timeout.timed_out(), "second worker did not start");
            drop(started);
        }
        if chunks[0] == "second" {
            let (started, started_signal) = &*self.second_started;
            *started.lock().expect("second-started lock") = true;
            started_signal.notify_one();

            let (released, release_signal) = &*self.release_second;
            let guard = released.lock().expect("release-second lock");
            let (guard, timeout) = release_signal
                .wait_timeout_while(guard, Duration::from_secs(2), |released| !*released)
                .expect("release wait");
            assert!(!timeout.timed_out(), "writer did not release second batch");
            drop(guard);
        }
        Ok(chunks.iter().map(|_| vec![1.0, 1.0]).collect())
    }

    fn embed_query(&self, _query: &str) -> Result<Vec<f32>, DocsError> {
        Ok(vec![1.0, 1.0])
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "coordinated"
    }
}

struct PanickingProvider;

impl LocalEmbeddingProvider for PanickingProvider {
    fn embed_chunks(&self, _chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        panic!("embedding worker panic")
    }

    fn embed_query(&self, _query: &str) -> Result<Vec<f32>, DocsError> {
        Ok(vec![1.0, 1.0])
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "panicking"
    }
}

struct RetryConcurrencyProvider {
    active: AtomicUsize,
    maximum: AtomicUsize,
    worker_started: Arc<(Mutex<bool>, Condvar)>,
    retry_started: Arc<(Mutex<bool>, Condvar)>,
    release_worker: Arc<(Mutex<bool>, Condvar)>,
}

impl RetryConcurrencyProvider {
    fn record_maximum(&self, active: usize) {
        self.maximum.fetch_max(active, Ordering::SeqCst);
    }
}

impl LocalEmbeddingProvider for RetryConcurrencyProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.record_maximum(active);
        match chunks[0].as_str() {
            "fail" => {
                self.active.fetch_sub(1, Ordering::SeqCst);
                Err(DocsError::Embedding {
                    message: "force individual retry".into(),
                })
            }
            "worker" => {
                let (started, signal) = &*self.worker_started;
                *started.lock().expect("worker-started lock") = true;
                signal.notify_one();
                let (released, signal) = &*self.release_worker;
                let released = released.lock().expect("release-worker lock");
                let (released, timeout) = signal
                    .wait_timeout_while(released, Duration::from_secs(2), |released| !*released)
                    .expect("release-worker wait");
                assert!(!timeout.timed_out(), "worker call was not released");
                drop(released);
                self.active.fetch_sub(1, Ordering::SeqCst);
                Ok(vec![vec![1.0, 1.0]])
            }
            "retry" => {
                let (started, signal) = &*self.retry_started;
                *started.lock().expect("retry-started lock") = true;
                signal.notify_one();
                self.active.fetch_sub(1, Ordering::SeqCst);
                Ok(vec![vec![1.0, 1.0]])
            }
            _ => unreachable!("unexpected test input"),
        }
    }

    fn embed_query(&self, _query: &str) -> Result<Vec<f32>, DocsError> {
        Ok(vec![1.0, 1.0])
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "retry-concurrency"
    }
}

#[test]
fn writer_retry_shares_provider_concurrency_limit() {
    let worker_started = Arc::new((Mutex::new(false), Condvar::new()));
    let retry_requested = Arc::new((Mutex::new(false), Condvar::new()));
    let retry_started = Arc::new((Mutex::new(false), Condvar::new()));
    let release_worker = Arc::new((Mutex::new(false), Condvar::new()));
    let provider = Arc::new(RetryConcurrencyProvider {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        worker_started: Arc::clone(&worker_started),
        retry_started: Arc::clone(&retry_started),
        release_worker: Arc::clone(&release_worker),
    });
    let batches = vec![
        EmbeddingBatch {
            index: 1,
            chunks: vec![chunk("a", "fail")],
        },
        EmbeddingBatch {
            index: 2,
            chunks: vec![chunk("b", "worker")],
        },
    ];
    let retry_wait = Arc::clone(&retry_requested);
    let releaser = std::thread::spawn(move || {
        let (started, signal) = &*worker_started;
        let started = started.lock().expect("worker-started lock");
        let (started, timeout) = signal
            .wait_timeout_while(started, Duration::from_secs(2), |started| !*started)
            .expect("worker-started wait");
        assert!(!timeout.timed_out(), "worker call did not start");
        drop(started);

        let (retry, signal) = &*retry_wait;
        let retry = retry.lock().expect("retry-requested lock");
        let (retry, timeout) = signal
            .wait_timeout_while(retry, Duration::from_secs(2), |retry| !*retry)
            .expect("retry-requested wait");
        assert!(!timeout.timed_out(), "writer did not request retry");
        drop(retry);

        let (retry, signal) = &*retry_started;
        let retry = retry.lock().expect("retry-started lock");
        let _ = signal
            .wait_timeout_while(retry, Duration::from_millis(100), |retry| !*retry)
            .expect("retry-started wait");
        let (released, signal) = &*release_worker;
        *released.lock().expect("release-worker lock") = true;
        signal.notify_one();
    });

    let retry_request = Arc::clone(&retry_requested);
    consume_embedded_batches(provider.clone(), batches, 1, 2, |embedded, provider| {
        if embedded.vectors.is_err() {
            let (requested, signal) = &*retry_request;
            *requested.lock().expect("retry-requested lock") = true;
            signal.notify_one();
            provider
                .embed_chunks(&["retry".into()])
                .expect("individual retry succeeds");
        }
    })
    .expect("embedding pipeline succeeds");
    releaser.join().expect("releaser thread succeeds");

    assert_eq!(provider.maximum.load(Ordering::SeqCst), 1);
}

struct CountingProvider {
    started: Arc<(Mutex<usize>, Condvar)>,
}

struct CountingBatches {
    next: usize,
    total: usize,
    admitted: Arc<AtomicUsize>,
}

impl Iterator for CountingBatches {
    type Item = EmbeddingBatch;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.total {
            return None;
        }
        self.next += 1;
        self.admitted.fetch_add(1, Ordering::SeqCst);
        Some(EmbeddingBatch {
            index: self.next,
            chunks: vec![chunk(&self.next.to_string(), "content")],
        })
    }
}

impl LocalEmbeddingProvider for CountingProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        let (started, signal) = &*self.started;
        *started.lock().expect("started-count lock") += 1;
        signal.notify_all();
        Ok(chunks.iter().map(|_| vec![1.0, 1.0]).collect())
    }

    fn embed_query(&self, _query: &str) -> Result<Vec<f32>, DocsError> {
        Ok(vec![1.0, 1.0])
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "counting"
    }
}

#[test]
fn source_admission_is_bounded_by_max_in_flight() {
    let started = Arc::new((Mutex::new(0), Condvar::new()));
    let admitted = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn LocalEmbeddingProvider> = Arc::new(CountingProvider {
        started: Arc::clone(&started),
    });
    let batches = CountingBatches {
        next: 0,
        total: 6,
        admitted: Arc::clone(&admitted),
    };
    let mut first = true;

    consume_embedded_batches(provider, batches, 2, 2, |_, _| {
        if first {
            first = false;
            assert_eq!(
                admitted.load(Ordering::SeqCst),
                2,
                "producer must not materialize batches beyond the residence bound"
            );
        }
    })
    .expect("embedding pipeline succeeds");

    assert_eq!(admitted.load(Ordering::SeqCst), 6);
}

#[test]
fn max_in_flight_includes_batches_waiting_for_writer() {
    let started = Arc::new((Mutex::new(0), Condvar::new()));
    let provider: Arc<dyn LocalEmbeddingProvider> = Arc::new(CountingProvider {
        started: Arc::clone(&started),
    });
    let batches: Vec<_> = (1..=4)
        .map(|index| EmbeddingBatch {
            index,
            chunks: vec![chunk(&index.to_string(), "content")],
        })
        .collect();
    let mut first = true;

    consume_embedded_batches(provider, batches, 2, 2, |_, _| {
        if first {
            first = false;
            let (count, signal) = &*started;
            let count = count.lock().expect("started-count lock");
            let (count, timeout) = signal
                .wait_timeout_while(count, Duration::from_secs(2), |count| *count < 2)
                .expect("initial-starts wait");
            assert!(!timeout.timed_out(), "two initial batches did not start");
            let (count, timeout) = signal
                .wait_timeout_while(count, Duration::from_millis(100), |count| *count <= 2)
                .expect("third-start wait");
            assert!(
                timeout.timed_out(),
                "third batch started while writer held a permit"
            );
            assert_eq!(*count, 2, "writer-held batch must consume one permit");
        }
    })
    .expect("embedding pipeline succeeds");
}

#[test]
fn completed_batch_is_consumed_before_other_worker_finishes() {
    let second_started = Arc::new((Mutex::new(false), Condvar::new()));
    let release_second = Arc::new((Mutex::new(false), Condvar::new()));
    let provider: Arc<dyn LocalEmbeddingProvider> = Arc::new(CoordinatedProvider {
        second_started: Arc::clone(&second_started),
        release_second: Arc::clone(&release_second),
    });
    let batches = vec![
        EmbeddingBatch {
            index: 1,
            chunks: vec![chunk("a", "first")],
        },
        EmbeddingBatch {
            index: 2,
            chunks: vec![chunk("b", "second")],
        },
    ];
    let release_from_writer = Arc::clone(&release_second);
    let caller_thread = std::thread::current().id();

    consume_embedded_batches(provider, batches, 2, 2, move |batch, _| {
        assert_eq!(std::thread::current().id(), caller_thread);
        if batch.index == 1 {
            let (released, signal) = &*release_from_writer;
            *released.lock().expect("release-second lock") = true;
            signal.notify_one();
        }
    })
    .expect("embedding pipeline succeeds");

    assert!(*second_started.0.lock().expect("second-started lock"));
}

#[test]
fn worker_panic_fails_the_embedding_pipeline() {
    let provider: Arc<dyn LocalEmbeddingProvider> = Arc::new(PanickingProvider);
    let batches = vec![EmbeddingBatch {
        index: 1,
        chunks: vec![chunk("a", "first")],
    }];

    let error = consume_embedded_batches(provider, batches, 2, 2, |_, _| {})
        .expect_err("worker panic must surface");

    assert!(error.to_string().contains("embedding worker panicked"));
}

#[test]
fn parallel_batches_preserve_batch_indices() {
    let provider: Arc<dyn LocalEmbeddingProvider> = Arc::new(MockProvider);
    let batches = vec![
        EmbeddingBatch {
            index: 2,
            chunks: vec![chunk("b", "second")],
        },
        EmbeddingBatch {
            index: 1,
            chunks: vec![chunk("a", "first")],
        },
    ];
    let mut results = Vec::new();

    consume_embedded_batches(provider, batches, 2, 2, |batch, _| results.push(batch))
        .expect("embedding pipeline succeeds");

    results.sort_by_key(|result| result.index);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].index, 1);
    assert_eq!(results[1].index, 2);
    assert_eq!(results[0].vectors.as_ref().unwrap()[0], vec![5.0, 1.0]);
}
