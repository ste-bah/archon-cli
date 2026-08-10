use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cozo::DbInstance;
use tokio::sync::{mpsc, oneshot};

const AUDIT_QUEUE_CAPACITY: usize = 256;

pub(super) enum SandboxAuditWrite {
    Session(Box<archon_learning::sandbox_sessions::SandboxSessionRecord>),
    RuntimeEvent {
        event: Box<archon_learning::sandbox_runtime_events::SandboxRuntimeEventRecord>,
        ledger: Option<Box<archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord>>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SandboxAuditReadback {
    pub accepted: u64,
    pub dropped: u64,
    pub persisted: u64,
    pub failed: u64,
}

struct SandboxAuditWriterState {
    accepting: bool,
    sender: mpsc::Sender<WorkerMessage>,
}

struct SandboxAuditCounters {
    accepted: AtomicU64,
    dropped: AtomicU64,
    persisted: AtomicU64,
    failed: AtomicU64,
}

impl Default for SandboxAuditCounters {
    fn default() -> Self {
        Self {
            accepted: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            persisted: AtomicU64::new(0),
            failed: AtomicU64::new(0),
        }
    }
}

#[derive(Clone)]
pub(super) struct SandboxAuditWriter {
    state: Arc<Mutex<SandboxAuditWriterState>>,
    counters: Arc<SandboxAuditCounters>,
}

#[derive(Clone)]
pub(crate) struct SandboxAuditDrainHandle {
    drain: Arc<tokio::sync::Mutex<Option<SandboxAuditDrain>>>,
}

impl SandboxAuditDrainHandle {
    pub fn new(drain: SandboxAuditDrain) -> Self {
        Self {
            drain: Arc::new(tokio::sync::Mutex::new(Some(drain))),
        }
    }

    #[cfg(test)]
    pub fn empty_for_test() -> Self {
        Self {
            drain: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    pub async fn shutdown(
        &self,
        timeout: Duration,
    ) -> anyhow::Result<Option<SandboxAuditReadback>> {
        let Some(drain) = self.drain.lock().await.take() else {
            return Ok(None);
        };
        drain.shutdown(timeout).await.map(Some)
    }
}

pub(crate) struct SandboxAuditDrain {
    writer: SandboxAuditWriter,
    worker: tokio::task::JoinHandle<()>,
}

enum WorkerMessage {
    Persist(SandboxAuditWrite),
    #[cfg(test)]
    Flush(oneshot::Sender<SandboxAuditReadback>),
    Shutdown(oneshot::Sender<SandboxAuditReadback>),
}

impl SandboxAuditWriter {
    pub fn new(db: Arc<DbInstance>) -> (Self, SandboxAuditDrain) {
        let (sender, receiver) = mpsc::channel(AUDIT_QUEUE_CAPACITY);
        let counters = Arc::new(SandboxAuditCounters::default());
        let writer = Self {
            state: Arc::new(Mutex::new(SandboxAuditWriterState {
                accepting: true,
                sender,
            })),
            counters,
        };
        let worker = archon_observability::spawn_named(
            "sandbox-audit-writer",
            run_writer(db, receiver, Arc::clone(&writer.counters)),
        );
        let drain = SandboxAuditDrain {
            writer: writer.clone(),
            worker,
        };
        (writer, drain)
    }

    pub fn enqueue(&self, write: SandboxAuditWrite) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !state.accepting {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("sandbox audit admission closed; evidence dropped");
            return;
        }
        match state.sender.try_send(WorkerMessage::Persist(write)) {
            Ok(()) => {
                self.counters.accepted.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) => {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(%error, "sandbox audit queue unavailable; evidence dropped");
            }
        }
    }

    fn close_admission(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .accepting = false;
    }

    #[cfg(test)]
    pub async fn flush_for_test(&self) -> SandboxAuditReadback {
        let (acknowledgement, receiver) = oneshot::channel();
        let sender = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .sender
            .clone();
        if sender
            .send(WorkerMessage::Flush(acknowledgement))
            .await
            .is_err()
        {
            return self.readback();
        }
        receiver.await.unwrap_or_else(|_| self.readback())
    }

    async fn shutdown(&self) -> SandboxAuditReadback {
        let (acknowledgement, receiver) = oneshot::channel();
        let sender = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .sender
            .clone();
        if sender
            .send(WorkerMessage::Shutdown(acknowledgement))
            .await
            .is_err()
        {
            return self.readback();
        }
        receiver.await.unwrap_or_else(|_| self.readback())
    }

    #[cfg(test)]
    fn bounded_for_test(capacity: usize) -> (Self, mpsc::Receiver<WorkerMessage>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let counters = Arc::new(SandboxAuditCounters::default());
        (
            Self {
                state: Arc::new(Mutex::new(SandboxAuditWriterState {
                    accepting: true,
                    sender,
                })),
                counters,
            },
            receiver,
        )
    }

    #[cfg(test)]
    pub fn readback_for_test(&self) -> SandboxAuditReadback {
        self.readback()
    }

    fn readback(&self) -> SandboxAuditReadback {
        readback(&self.counters)
    }
}

impl SandboxAuditDrain {
    pub async fn shutdown(self, timeout: Duration) -> anyhow::Result<SandboxAuditReadback> {
        self.writer.close_admission();
        let counters = Arc::clone(&self.writer.counters);
        let drain_counters = Arc::clone(&counters);
        let drain = async move {
            let shutdown_readback = self.writer.shutdown().await;
            if let Err(error) = self.worker.await {
                let readback = readback(&drain_counters);
                let unresolved = unresolved(&readback);
                anyhow::bail!(
                    "sandbox audit writer task failed: {error}; accepted={}; persisted={}; failed={}; dropped={}; unresolved={unresolved}",
                    readback.accepted,
                    readback.persisted,
                    readback.failed,
                    readback.dropped,
                );
            }
            Ok::<SandboxAuditReadback, anyhow::Error>(shutdown_readback)
        };
        let readback = tokio::time::timeout(timeout, drain).await.map_err(|_| {
            let readback = readback(&counters);
            let unresolved = unresolved(&readback);
            anyhow::anyhow!(
                "sandbox audit drain timed out after {timeout:?}; accepted={}; persisted={}; failed={}; dropped={}; unresolved={unresolved}",
                readback.accepted,
                readback.persisted,
                readback.failed,
                readback.dropped,
            )
        })??;
        let unresolved = unresolved(&readback);
        if readback.dropped > 0 || readback.failed > 0 || unresolved > 0 {
            anyhow::bail!(
                "sandbox audit evidence incomplete; accepted={}; persisted={}; failed={}; dropped={}; unresolved={unresolved}",
                readback.accepted,
                readback.persisted,
                readback.failed,
                readback.dropped,
            );
        }
        Ok(readback)
    }
}

fn unresolved(readback: &SandboxAuditReadback) -> u64 {
    readback
        .accepted
        .saturating_sub(readback.persisted.saturating_add(readback.failed))
}

async fn run_writer(
    db: Arc<DbInstance>,
    mut receiver: mpsc::Receiver<WorkerMessage>,
    counters: Arc<SandboxAuditCounters>,
) {
    while let Some(message) = receiver.recv().await {
        match message {
            WorkerMessage::Persist(write) => {
                let db = Arc::clone(&db);
                let result = archon_observability::spawn_blocking_named(
                    "sandbox-audit-persist",
                    move || persist_write(&db, write),
                )
                .await;
                match result {
                    Ok(Ok(())) => {
                        counters.persisted.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(Err(error)) => {
                        counters.failed.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(%error, "sandbox audit persistence failed");
                    }
                    Err(error) => {
                        counters.failed.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(%error, "sandbox audit persistence task failed");
                    }
                }
            }
            #[cfg(test)]
            WorkerMessage::Flush(acknowledgement) => {
                let _ = acknowledgement.send(readback(&counters));
            }
            WorkerMessage::Shutdown(acknowledgement) => {
                let _ = acknowledgement.send(readback(&counters));
                break;
            }
        }
    }
}

fn persist_write(db: &DbInstance, write: SandboxAuditWrite) -> anyhow::Result<()> {
    match write {
        SandboxAuditWrite::Session(session) => {
            archon_learning::sandbox_sessions::insert_sandbox_session(db, &session)?;
        }
        SandboxAuditWrite::RuntimeEvent { event, ledger } => {
            archon_learning::sandbox_runtime_events::insert_sandbox_runtime_event_with_ledger(
                db,
                &event,
                ledger.as_deref(),
            )?;
        }
    }
    Ok(())
}

fn readback(counters: &SandboxAuditCounters) -> SandboxAuditReadback {
    SandboxAuditReadback {
        accepted: counters.accepted.load(Ordering::Relaxed),
        dropped: counters.dropped.load(Ordering::Relaxed),
        persisted: counters.persisted.load(Ordering::Relaxed),
        failed: counters.failed.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str) -> SandboxAuditWrite {
        SandboxAuditWrite::Session(Box::new(
            archon_learning::sandbox_sessions::SandboxSessionRecord::new(
                id,
                "local",
                "test-profile",
                "configured",
                chrono::Utc::now().to_rfc3339(),
            ),
        ))
    }

    #[tokio::test]
    async fn worker_panic_reports_exact_unresolved_accounting() {
        let (writer, mut receiver) = SandboxAuditWriter::bounded_for_test(2);
        writer.enqueue(session("panic"));
        let worker = tokio::spawn(async move {
            assert!(matches!(
                receiver.recv().await,
                Some(WorkerMessage::Persist(_))
            ));
            panic!("worker panic");
        });
        let drain = SandboxAuditDrain { writer, worker };

        let error = drain
            .shutdown(Duration::from_secs(1))
            .await
            .expect_err("panicked audit worker must fail loudly");
        let message = error.to_string();

        assert!(message.contains("task failed"), "{error:#}");
        assert!(message.contains("accepted=1"), "{error:#}");
        assert!(message.contains("persisted=0"), "{error:#}");
        assert!(message.contains("failed=0"), "{error:#}");
        assert!(message.contains("dropped=0"), "{error:#}");
        assert!(message.contains("unresolved=1"), "{error:#}");
    }

    #[tokio::test]
    async fn persistence_failure_reports_exact_terminal_accounting() {
        let db = crate::command::test_support::registered_learning_test_db(
            "test-sandbox-audit-writer-failure",
        );
        db.run_script(
            "{::remove sandbox_sessions}",
            Default::default(),
            cozo::ScriptMutability::Mutable,
        )
        .unwrap();
        let (writer, drain) = SandboxAuditWriter::new(db.arc());
        writer.enqueue(session("failure"));

        let error = drain
            .shutdown(Duration::from_secs(2))
            .await
            .expect_err("failed audit persistence must fail shutdown");
        let message = error.to_string();

        assert!(message.contains("accepted=1"), "{error:#}");
        assert!(message.contains("persisted=0"), "{error:#}");
        assert!(message.contains("failed=1"), "{error:#}");
        assert!(message.contains("dropped=0"), "{error:#}");
        assert!(message.contains("unresolved=0"), "{error:#}");
    }

    #[tokio::test]
    async fn stalled_drain_returns_unresolved_count() {
        let (writer, _receiver) = SandboxAuditWriter::bounded_for_test(1);
        writer.enqueue(session("stalled"));
        let worker = tokio::spawn(std::future::pending());
        let drain = SandboxAuditDrain { writer, worker };

        let error = drain
            .shutdown(Duration::from_millis(10))
            .await
            .expect_err("stalled audit drain must fail within its deadline");

        assert!(error.to_string().contains("unresolved=1"), "{error:#}");
    }

    #[tokio::test]
    async fn shutdown_persists_accepted_evidence_before_returning() {
        let db = crate::command::test_support::registered_learning_test_db(
            "test-sandbox-audit-writer-shutdown",
        );
        let (writer, drain) = SandboxAuditWriter::new(db.clone());
        writer.enqueue(session("shutdown-session"));

        let readback = drain.shutdown(Duration::from_secs(2)).await.unwrap();
        let sessions =
            archon_learning::sandbox_sessions::list_sandbox_sessions_by_status(&db, "configured")
                .unwrap();

        assert_eq!(readback.accepted, 1);
        assert_eq!(readback.persisted, 1);
        assert_eq!(readback.dropped, 0);
        assert_eq!(readback.failed, 0);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].sandbox_session_id, "shutdown-session");
    }

    #[test]
    fn closed_admission_rejects_late_evidence() {
        let (writer, _receiver) = SandboxAuditWriter::bounded_for_test(1);
        writer.close_admission();

        writer.enqueue(session("late"));

        assert_eq!(
            writer.readback_for_test(),
            SandboxAuditReadback {
                accepted: 0,
                dropped: 1,
                persisted: 0,
                failed: 0,
            }
        );
    }

    #[test]
    fn full_queue_drops_evidence_without_blocking_producer() {
        let (writer, _receiver) = SandboxAuditWriter::bounded_for_test(1);

        writer.enqueue(session("first"));
        writer.enqueue(session("second"));

        assert_eq!(
            writer.readback_for_test(),
            SandboxAuditReadback {
                accepted: 1,
                dropped: 1,
                persisted: 0,
                failed: 0,
            }
        );
    }
}
