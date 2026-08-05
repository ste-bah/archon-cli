// Regression tests for the per-turn memory injection stall.
//
// `inject_memories` used to be a synchronous fn called straight from an async
// fn. It has no `.await` points, so the tokio task never yielded: it pinned a
// worker thread for the whole (potentially unbounded) scan and the cancellation
// token set by the caller was never polled, which is why Ctrl+C could not
// interrupt it. Every test here fails against that shape.

use archon_memory::types::{
    Memory, MemoryError, MemoryType, RelType, SearchFilter, StoreMemoryOutcome,
};
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::mpsc;

/// Memory backend that can be made slow, or blocked outright, on demand.
struct BlockingMemory {
    /// Number of `recall_memories` calls that actually reached the backend.
    recalls: AtomicUsize,
    /// Wall-clock cost of each recall, simulating a large store.
    recall_delay: std::time::Duration,
    /// When present, the FIRST recall parks here until the test releases it.
    first_recall_gate: Option<std::sync::Mutex<mpsc::Receiver<()>>>,
    /// Fired once the correction search runs, which is strictly after the
    /// injector's mutex guard has been dropped.
    finished: Arc<tokio::sync::Notify>,
    /// Number of correction searches that reached the backend. Corrections are
    /// recalled through `search_memories`, so this counts the work the per-turn
    /// cache is supposed to eliminate.
    searches: AtomicUsize,
}

impl BlockingMemory {
    fn slow(recall_delay: std::time::Duration) -> Self {
        Self {
            recalls: AtomicUsize::new(0),
            recall_delay,
            first_recall_gate: None,
            finished: Arc::new(tokio::sync::Notify::new()),
            searches: AtomicUsize::new(0),
        }
    }

    fn gated(gate: mpsc::Receiver<()>) -> Self {
        Self {
            recalls: AtomicUsize::new(0),
            recall_delay: std::time::Duration::ZERO,
            first_recall_gate: Some(std::sync::Mutex::new(gate)),
            finished: Arc::new(tokio::sync::Notify::new()),
            searches: AtomicUsize::new(0),
        }
    }

    fn recall_count(&self) -> usize {
        self.recalls.load(AtomicOrdering::SeqCst)
    }

    fn search_count(&self) -> usize {
        self.searches.load(AtomicOrdering::SeqCst)
    }
}

impl MemoryTrait for BlockingMemory {
    fn store_memory(
        &self,
        _content: &str,
        _title: &str,
        _memory_type: MemoryType,
        _importance: f64,
        _tags: &[String],
        _source_type: &str,
        _project_path: &str,
    ) -> Result<String, MemoryError> {
        unimplemented!("BlockingMemory: store_memory not used by memory injection")
    }

    fn store_memory_with_id_outcome(
        &self,
        _id: &str,
        _content: &str,
        _title: &str,
        _memory_type: MemoryType,
        _importance: f64,
        _tags: &[String],
        _source_type: &str,
        _project_path: &str,
    ) -> Result<StoreMemoryOutcome, MemoryError> {
        unimplemented!("BlockingMemory: store_memory_with_id_outcome not used by memory injection")
    }

    fn store_memory_with_id(
        &self,
        _id: &str,
        _content: &str,
        _title: &str,
        _memory_type: MemoryType,
        _importance: f64,
        _tags: &[String],
        _source_type: &str,
        _project_path: &str,
    ) -> Result<Memory, MemoryError> {
        unimplemented!("BlockingMemory: store_memory_with_id not used by memory injection")
    }

    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        Err(MemoryError::NotFound(id.to_string()))
    }

    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError> {
        Err(MemoryError::NotFound(id.to_string()))
    }

    fn update_memory(
        &self,
        _id: &str,
        _content: Option<&str>,
        _tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        unimplemented!("BlockingMemory: update_memory not used by memory injection")
    }

    fn apply_importance_delta(
        &self,
        _id: &str,
        _delta: f64,
        _provenance_id: &str,
    ) -> Result<Memory, MemoryError> {
        unimplemented!("BlockingMemory: apply_importance_delta not used by memory injection")
    }

    fn has_importance_application(
        &self,
        _memory_id: &str,
        _provenance_id: &str,
    ) -> Result<bool, MemoryError> {
        unimplemented!("BlockingMemory: has_importance_application not used by memory injection")
    }

    fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
        unimplemented!("BlockingMemory: delete_memory not used by memory injection")
    }

    fn create_relationship(
        &self,
        _from_id: &str,
        _to_id: &str,
        _rel_type: RelType,
        _context: Option<&str>,
        _strength: f64,
    ) -> Result<(), MemoryError> {
        unimplemented!("BlockingMemory: create_relationship not used by memory injection")
    }

    fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        let call = self.recalls.fetch_add(1, AtomicOrdering::SeqCst);
        if call == 0
            && let Some(ref gate) = self.first_recall_gate
        {
            let _ = gate.lock().expect("gate").recv();
        }
        if !self.recall_delay.is_zero() {
            std::thread::sleep(self.recall_delay);
        }
        Ok(vec![Memory {
            id: "mem-1".into(),
            content: "the deployment pipeline is gated on review".into(),
            title: "deployment".into(),
            memory_type: MemoryType::Fact,
            importance: 0.9,
            tags: vec!["deployment".into()],
            source_type: "test".into(),
            project_path: "/project".into(),
            created_at: chrono::Utc::now(),
            updated_at: None,
            access_count: 0,
            last_accessed: None,
        }])
    }

    fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        self.searches.fetch_add(1, AtomicOrdering::SeqCst);
        self.finished.notify_one();
        Ok(Vec::new())
    }

    fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        Ok(Vec::new())
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        Ok(0)
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        Ok(0)
    }

    fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
        Ok(Vec::new())
    }
}

fn agent_with_memory(memory: Arc<BlockingMemory>) -> Agent {
    let mut agent = test_agent();
    agent.set_memory(memory);
    agent
        .state
        .add_user_message("how does the deployment pipeline gating work");
    agent
}

/// The cancellation token was never observed, because the fn contained no
/// await point at which it could be. Pre-cancelled, the scan still ran.
#[tokio::test(flavor = "current_thread")]
async fn injection_observes_an_already_cancelled_token() {
    let memory = Arc::new(BlockingMemory::slow(std::time::Duration::ZERO));
    let mut agent = agent_with_memory(Arc::clone(&memory));

    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    agent.set_cancel_token(Some(token));

    let system = agent.inject_memories().await;

    assert_eq!(
        memory.recall_count(),
        0,
        "a cancelled turn must not start the memory scan"
    );
    assert!(
        !system
            .iter()
            .any(|block| block["text"].as_str().is_some_and(|t| t.contains("<memories>"))),
        "nothing should be injected for a cancelled turn"
    );
}

/// The scan must not run on the async executor. On a current-thread runtime a
/// concurrently spawned task can only make progress if the injection yields —
/// which it does not unless the blocking work is off-executor.
#[tokio::test(flavor = "current_thread")]
async fn injection_does_not_pin_the_async_executor() {
    let memory = Arc::new(BlockingMemory::slow(std::time::Duration::from_millis(150)));
    let mut agent = agent_with_memory(Arc::clone(&memory));

    let progressed = Arc::new(AtomicUsize::new(0));
    let progressed_task = Arc::clone(&progressed);
    tokio::spawn(async move {
        progressed_task.fetch_add(1, AtomicOrdering::SeqCst);
    });

    let system = agent.inject_memories().await;

    assert_eq!(
        progressed.load(AtomicOrdering::SeqCst),
        1,
        "another task must be able to run while memory recall is in flight; \
         a synchronous call from an async fn never yields the executor"
    );
    assert!(
        system
            .iter()
            .any(|block| block["text"].as_str().is_some_and(|t| t.contains("<memories>"))),
        "the injection still has to produce its block"
    );
}

/// `spawn_blocking` work cannot be cancelled, so anything moved into it is lost
/// when the caller stops waiting. The injector must therefore survive a
/// cancelled injection intact, cache included — if it were moved in and only
/// restored on the success path, it would be silently reset for the rest of the
/// session.
#[tokio::test(flavor = "current_thread")]
async fn cancelled_injection_leaves_the_injector_usable() {
    let (release, gate) = mpsc::channel();
    let memory = Arc::new(BlockingMemory::gated(gate));
    let finished = Arc::clone(&memory.finished);
    let mut agent = agent_with_memory(Arc::clone(&memory));

    let token = tokio_util::sync::CancellationToken::new();
    agent.set_cancel_token(Some(token.clone()));
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        token.cancel();
    });

    let cancelled = agent.inject_memories().await;
    assert!(
        !cancelled
            .iter()
            .any(|block| block["text"].as_str().is_some_and(|t| t.contains("<memories>"))),
        "the cancelled turn returns without a memory block"
    );
    cancel_task.await.expect("cancel task");

    // Let the abandoned scan finish and wait until it has published its result
    // into the injector and dropped the guard.
    release.send(()).expect("release the abandoned scan");
    finished.notified().await;

    agent.set_cancel_token(None);
    let system = agent.inject_memories().await;

    assert!(
        system
            .iter()
            .any(|block| block["text"].as_str().is_some_and(|t| t.contains("<memories>"))),
        "the injector must still work after a cancelled injection"
    );
    assert_eq!(
        memory.recall_count(),
        1,
        "the injector's cache must survive cancellation; a second backend \
         recall means it was reset to a fresh injector"
    );
}

/// Corrections are recalled once per TURN, not once per agent-loop iteration.
///
/// `inject_memories` runs on every iteration. Injection memoises by context
/// hash, but `recall_corrections` had no cache at all -- measured at 8.8s per
/// call returning zero rows on a 1.7 GB store, so a twenty-round turn re-paid
/// ~176 seconds answering the same question. Against that shape this fails with
/// `left: 3`.
///
/// A NEW turn must still recall: the cache is keyed on `turn_number` precisely
/// so a correction recorded at the end of one turn is visible in the next.
#[tokio::test(flavor = "current_thread")]
async fn corrections_are_recalled_once_per_turn_not_once_per_round() {
    let memory = Arc::new(BlockingMemory::slow(std::time::Duration::ZERO));
    let mut agent = agent_with_memory(Arc::clone(&memory));

    for _ in 0..3 {
        let _ = agent.inject_memories().await;
    }
    assert_eq!(
        memory.search_count(),
        1,
        "three rounds of one turn must recall corrections once"
    );

    agent.turn_number += 1; // what `begin_process_turn` does at the top of a turn
    let _ = agent.inject_memories().await;
    assert_eq!(
        memory.search_count(),
        2,
        "a new turn must recall again, or a correction recorded at the end of          the previous turn would never be seen"
    );
}
