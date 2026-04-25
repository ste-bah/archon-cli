use archon_tui_test_support::fake_registry::{
    FakeRegistry, PollStatus, spawn_fake_subagent, spawn_n_fake_subagents,
};
use std::time::Duration;
use tokio::sync::oneshot;

fn rt_multi() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("rt")
}

#[test]
fn insert_then_poll_returns_done_after_task_completes() {
    let rt = rt_multi();
    rt.block_on(async {
        let reg = FakeRegistry::new();
        let _latency = spawn_fake_subagent(&reg, "t1", |tx: oneshot::Sender<String>| {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(5)).await;
                let _ = tx.send("done".to_string());
            })
        });
        // wait for completion, poll until Done
        let mut saw_done = false;
        for _ in 0..200 {
            if let PollStatus::Done(msg) = reg.poll("t1") {
                assert_eq!(msg, "done");
                saw_done = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(saw_done, "expected PollStatus::Done within 1s");
        assert_eq!(reg.len(), 1);
    });
}

#[test]
fn poll_unknown_id_returns_notfound() {
    let rt = rt_multi();
    rt.block_on(async {
        let reg = FakeRegistry::new();
        assert_eq!(reg.poll("does-not-exist"), PollStatus::NotFound);
        assert_eq!(reg.len(), 0);
    });
}

#[test]
fn concurrent_100_inserts_all_under_10ms() {
    let rt = rt_multi();
    rt.block_on(async {
        let reg = FakeRegistry::new();
        let latencies = spawn_n_fake_subagents(&reg, 100);
        assert_eq!(latencies.len(), 100);
        assert_eq!(reg.len(), 100);
        let worst = latencies.iter().copied().max().unwrap();
        // NFR-TUI-SUB-001: <10ms spawn latency; 3x CI tolerance = 30ms
        assert!(
            worst < Duration::from_millis(30),
            "worst insert latency {:?} exceeds 30ms",
            worst
        );
    });
}

#[test]
fn await_all_times_out_when_task_stuck() {
    let rt = rt_multi();
    rt.block_on(async {
        let reg = FakeRegistry::new();
        // Script that never sends on the oneshot — classic stuck task.
        // The async move MUST capture tx so the sender stays alive for the
        // duration of the timeout budget; otherwise dropping tx closes rx
        // and await_all reports `pending` instead of `timed_out`.
        let _ = spawn_fake_subagent(&reg, "stuck", |tx: oneshot::Sender<String>| {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                // Touch tx so it is moved into the async block and kept alive.
                drop(tx);
            })
        });
        let report = reg.await_all(Duration::from_millis(50)).await;
        assert!(
            report.timed_out > 0,
            "expected at least one timed-out task, got {:?}",
            report
        );
        reg.shutdown_all();
    });
}

#[test]
fn shutdown_all_aborts_pending_handles() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let rt = rt_multi();
    rt.block_on(async {
        let reg = FakeRegistry::new();
        let completed = Arc::new(AtomicUsize::new(0));
        for i in 0..10 {
            let completed_clone = Arc::clone(&completed);
            spawn_fake_subagent(
                &reg,
                format!("long-{i}"),
                move |_tx: oneshot::Sender<String>| {
                    Box::pin(async move {
                        // Short enough that a no-op shutdown_all would let the task finish
                        // naturally within the post-shutdown wait window, exposing the bug.
                        tokio::time::sleep(Duration::from_millis(30)).await;
                        completed_clone.fetch_add(1, Ordering::SeqCst);
                    })
                },
            );
        }
        assert_eq!(reg.len(), 10);
        reg.shutdown_all();
        // Wait long enough that, if shutdown_all were a no-op, all 10 tasks
        // would reach their fetch_add and increment the counter.
        tokio::time::sleep(Duration::from_millis(150)).await;
        let finished = completed.load(Ordering::SeqCst);
        assert_eq!(
            finished, 0,
            "expected 0 natural completions after shutdown_all, got {finished}"
        );
        // Entries remain in the map — abort does not remove, only cancels.
        assert_eq!(reg.len(), 10);
    });
}
