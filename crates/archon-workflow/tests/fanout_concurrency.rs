//! Regression: fan-out width must respect a runner's hard concurrency cap.
//!
//! Reproduces the live failure where 22/26 fan-out items died instantly with
//! "max concurrent subagents reached (4)". The fan-out semaphore admitted more
//! concurrent items than the subagent-backed runner could accept, so overflow
//! items were hard-rejected as terminal failures instead of waiting for a slot.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_workflow::{
    StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

fn many_item_spec() -> WorkflowSpec {
    // discover emits N items; review fans out one branch per item. The item
    // documents are produced dynamically by the runner below.
    WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: fanout-cap-test
task: exercise fan-out concurrency clamp
max_parallelism: 8
stages:
  - id: discover
    kind: agent
    agent: workflow-discovery
    outputs: [items]
  - id: review
    kind: fanout
    agent: workflow-reviewer
    foreach: ${discover.items}
    depends_on: [discover]
"#,
    )
    .unwrap()
}

/// A runner that hard-rejects work beyond `cap` concurrent stages — exactly the
/// behaviour of the subagent manager in the live path — and records the peak
/// observed concurrency so the test can prove the clamp held.
struct CappedRunner {
    cap: usize,
    item_count: usize,
    in_flight: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl WorkflowStageRunner for CappedRunner {
    fn max_concurrency(&self) -> Option<usize> {
        Some(self.cap)
    }

    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        if request.stage_id == "discover" {
            let items: Vec<String> = (0..self.item_count)
                .map(|i| format!(r#"{{"unit":"u{i}"}}"#))
                .collect();
            return Ok(StageRunOutput::markdown(format!(
                r#"{{"items":[{}]}}"#,
                items.join(",")
            )));
        }

        // Fan-out branch: enforce the hard cap. If the executor admitted more
        // than `cap` concurrent items this returns an error, mirroring the
        // "max concurrent subagents reached" failure.
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        if now > self.cap {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            return Err(archon_workflow::WorkflowError::StageFailed(format!(
                "max concurrent reached ({})",
                self.cap
            )));
        }
        // Hold the slot briefly so genuine overlap can occur.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(StageRunOutput::markdown("reviewed"))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fanout_width_respects_runner_concurrency_cap() {
    let cap = 4;
    let item_count = 26; // mirrors the live run wf-0005a01f (26 items, cap 4).
    let in_flight = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let executor = WorkflowExecutor::new(store.clone(), WorkflowPolicy::default());
    let run = executor.start(many_item_spec()).unwrap();
    let run_id = run.id.clone();

    let runner = CappedRunner {
        cap,
        item_count,
        in_flight: in_flight.clone(),
        peak: peak.clone(),
    };
    let report = executor.execute_with_runner(run, &runner).await.unwrap();

    // No item should fail: the clamp must keep in-flight <= cap so the runner
    // never rejects. Before the fix, 22/26 items failed here.
    assert_eq!(
        report.failed,
        0,
        "fan-out items must not be hard-rejected; peak concurrency was {}",
        peak.load(Ordering::SeqCst)
    );
    assert!(
        peak.load(Ordering::SeqCst) <= cap,
        "peak concurrency {} exceeded cap {cap}",
        peak.load(Ordering::SeqCst)
    );

    let finished = store.load_state(&run_id).unwrap();
    assert_eq!(
        finished.stages.get("review").unwrap().status,
        StageStatus::Accepted,
        "review fan-out stage must be accepted"
    );
}
