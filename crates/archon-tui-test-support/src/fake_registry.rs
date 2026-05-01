//! fake_registry — deterministic FakeRegistry for phase-4 subagent spawn tests.
//!
//! Mirrors the shape of the real `archon_tools::background::BACKGROUND_AGENTS`
//! static without touching production code. Phase-4 tests consume this to
//! exercise 100-concurrent spawn/poll/shutdown semantics.

use dashmap::DashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct FakeBackgroundAgent {
    pub id: String,
    pub started_at: Instant,
    pub handle: JoinHandle<()>,
    pub done_rx: Mutex<Option<oneshot::Receiver<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollStatus {
    Running,
    Done(String),
    NotFound,
}

#[derive(Debug, Clone, Default)]
pub struct AwaitAllReport {
    pub completed: usize,
    pub pending: usize,
    pub timed_out: usize,
}

pub struct FakeRegistry {
    inner: Arc<DashMap<String, FakeBackgroundAgent>>,
}

impl FakeRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    pub fn insert(
        &self,
        id: impl Into<String>,
        handle: JoinHandle<()>,
        done_rx: oneshot::Receiver<String>,
    ) -> Duration {
        let id = id.into();
        let t0 = Instant::now();
        let agent = FakeBackgroundAgent {
            id: id.clone(),
            started_at: t0,
            handle,
            done_rx: Mutex::new(Some(done_rx)),
        };
        self.inner.insert(id, agent);
        t0.elapsed()
    }

    pub fn poll(&self, id: &str) -> PollStatus {
        let Some(entry) = self.inner.get(id) else {
            return PollStatus::NotFound;
        };
        let mut slot = entry.done_rx.lock().expect("done_rx mutex");
        if let Some(rx) = slot.as_mut() {
            match rx.try_recv() {
                Ok(msg) => {
                    *slot = None;
                    PollStatus::Done(msg)
                }
                Err(oneshot::error::TryRecvError::Empty) => PollStatus::Running,
                Err(oneshot::error::TryRecvError::Closed) => PollStatus::Running,
            }
        } else {
            // receiver was already consumed previously; treat as Done with empty marker
            PollStatus::Done(String::new())
        }
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn shutdown_all(&self) {
        for entry in self.inner.iter() {
            entry.handle.abort();
        }
    }

    pub async fn await_all(&self, timeout: Duration) -> AwaitAllReport {
        let deadline = Instant::now() + timeout;
        let mut report = AwaitAllReport::default();
        let ids: Vec<String> = self.inner.iter().map(|e| e.key().clone()).collect();
        for id in ids {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                report.timed_out += 1;
                continue;
            }
            match self.poll(&id) {
                PollStatus::Done(_) => {
                    report.completed += 1;
                    continue;
                }
                PollStatus::NotFound => continue,
                PollStatus::Running => {}
            }
            // Try to observe completion within remaining budget
            let rx_opt = {
                let Some(entry) = self.inner.get(&id) else {
                    continue;
                };
                let mut slot = entry.done_rx.lock().expect("done_rx mutex");
                slot.take()
            };
            if let Some(rx) = rx_opt {
                match tokio::time::timeout(remaining, rx).await {
                    Ok(Ok(_msg)) => report.completed += 1,
                    Ok(Err(_)) => report.pending += 1,
                    Err(_) => report.timed_out += 1,
                }
            }
        }
        report
    }
}

impl Default for FakeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn a fake subagent with a user-supplied closure that produces the body future.
/// Closure receives the oneshot sender and MUST signal completion via `tx.send(msg)`.
pub fn spawn_fake_subagent<F>(registry: &FakeRegistry, id: impl Into<String>, script: F) -> Duration
where
    F: FnOnce(oneshot::Sender<String>) -> BoxFuture<'static, ()> + Send + 'static,
{
    let (tx, rx) = oneshot::channel::<String>();
    let fut = script(tx);
    let handle = tokio::spawn(fut);
    registry.insert(id, handle, rx)
}

/// Spawn `n` trivial fake subagents that complete immediately with a sentinel message.
/// Returns per-insert latencies for NFR-TUI-SUB-001 assertions.
pub fn spawn_n_fake_subagents(registry: &FakeRegistry, n: usize) -> Vec<Duration> {
    let mut latencies = Vec::with_capacity(n);
    for i in 0..n {
        let id = format!("fake-{i}");
        let d = spawn_fake_subagent(registry, id, |tx: oneshot::Sender<String>| {
            Box::pin(async move {
                let _ = tx.send("done".to_string());
            })
        });
        latencies.push(d);
    }
    latencies
}
