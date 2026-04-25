//! Tests for PerAgentTaskQueue (TASK-AGS-205).

use std::time::Duration;

use archon_core::tasks::models::{TaskError, TaskId};
use archon_core::tasks::queue::{PerAgentTaskQueue, QueueConfig, TaskQueue};

fn make_queue(max_concurrent: usize, capacity: usize) -> PerAgentTaskQueue {
    PerAgentTaskQueue::new(QueueConfig {
        max_concurrent,
        queue_capacity: capacity,
        burst_capacity: 5,
        burst_threshold: Duration::from_secs(30),
    })
}

fn make_burst_queue(
    max_concurrent: usize,
    capacity: usize,
    burst_capacity: usize,
    burst_threshold: Duration,
) -> PerAgentTaskQueue {
    PerAgentTaskQueue::new(QueueConfig {
        max_concurrent,
        queue_capacity: capacity,
        burst_capacity,
        burst_threshold,
    })
}

/// Enqueue `n` tasks for the given agent, returning the generated TaskIds.
fn enqueue_n(queue: &PerAgentTaskQueue, agent: &str, n: usize) -> Vec<TaskId> {
    (0..n)
        .map(|_| {
            let id = TaskId::new();
            queue.enqueue(id, agent).unwrap();
            id
        })
        .collect()
}

#[test]
fn test_enqueue_respects_max_concurrent() {
    let queue = make_queue(10, 1000);
    let _ids = enqueue_n(&queue, "agent-a", 20);

    // First 10 should get permits (semaphore has 10).
    let mut dequeued = Vec::new();
    for _ in 0..10 {
        let result = queue.try_dequeue("agent-a");
        assert!(result.is_some(), "expected permit for first 10 tasks");
        dequeued.push(result.unwrap());
    }

    // 11th should return None — all permits exhausted, remaining 10 still pending.
    assert!(
        queue.try_dequeue("agent-a").is_none(),
        "expected None when all permits are taken"
    );

    // Pending depth should be 10 (20 enqueued - 10 dequeued).
    assert_eq!(queue.depth("agent-a"), 10);
}

#[test]
fn test_enqueue_queue_full_returns_error() {
    let queue = make_queue(10, 5);
    enqueue_n(&queue, "agent-a", 5);

    let sixth = TaskId::new();
    let result = queue.enqueue(sixth, "agent-a");
    assert!(
        matches!(result, Err(TaskError::QueueFull)),
        "expected QueueFull, got {:?}",
        result
    );
}

#[test]
fn test_queue_full_then_slot_frees() {
    let queue = make_queue(2, 5);
    enqueue_n(&queue, "agent-a", 5);

    // Dequeue 2 (uses all permits).
    let first = queue.try_dequeue("agent-a").expect("should dequeue 1st");
    let second = queue.try_dequeue("agent-a").expect("should dequeue 2nd");

    // No more permits.
    assert!(queue.try_dequeue("agent-a").is_none());

    // Depth is 3 (5 enqueued - 2 dequeued).
    assert_eq!(queue.depth("agent-a"), 3);

    // Drop one permit to simulate task completion.
    drop(first);

    // Now one permit is free — oldest pending should become dequeueable.
    let third = queue.try_dequeue("agent-a");
    assert!(third.is_some(), "expected slot to free after permit drop");

    // Depth is now 2.
    assert_eq!(queue.depth("agent-a"), 2);

    // Drop second permit.
    drop(second);

    let fourth = queue.try_dequeue("agent-a");
    assert!(fourth.is_some(), "expected another slot to free");
    assert_eq!(queue.depth("agent-a"), 1);
}

#[test]
fn test_remove_pending_cancels_before_execution() {
    let queue = make_queue(2, 100);
    let ids = enqueue_n(&queue, "agent-a", 5);

    // Dequeue the first 2 (max_concurrent = 2).
    let _p1 = queue.try_dequeue("agent-a").unwrap();
    let _p2 = queue.try_dequeue("agent-a").unwrap();

    // ids[2], ids[3], ids[4] are still pending. Remove ids[3].
    let removed = queue.remove_pending(ids[3]);
    assert!(removed, "expected remove_pending to find ids[3]");

    // Depth should be 2 (was 3, removed 1).
    assert_eq!(queue.depth("agent-a"), 2);

    // Drop permits and dequeue remaining — ids[3] should NOT appear.
    drop(_p1);
    drop(_p2);

    let mut dequeued_ids = Vec::new();
    while let Some((tid, _permit)) = queue.try_dequeue("agent-a") {
        dequeued_ids.push(tid);
    }

    assert!(
        !dequeued_ids.contains(&ids[3]),
        "removed task should not be dequeued"
    );
    assert!(
        dequeued_ids.contains(&ids[2]),
        "ids[2] should still be present"
    );
    assert!(
        dequeued_ids.contains(&ids[4]),
        "ids[4] should still be present"
    );
}

#[test]
fn test_burst_mode_activates_after_threshold() {
    // max_concurrent=2, burst_capacity=3, burst_threshold=50ms
    let queue = make_burst_queue(2, 100, 3, Duration::from_millis(50));

    // Enqueue 5 tasks.
    let _ids = enqueue_n(&queue, "agent-a", 5);

    // Dequeue first 2 (normal permits).
    let _p1 = queue.try_dequeue("agent-a").expect("1st normal permit");
    let _p2 = queue.try_dequeue("agent-a").expect("2nd normal permit");

    // No more normal permits.
    assert!(
        queue.try_dequeue("agent-a").is_none(),
        "should not get permit before burst threshold"
    );

    // Wait past the burst threshold.
    std::thread::sleep(Duration::from_millis(60));

    // Now burst permits should be available for the remaining 3 pending tasks.
    let b1 = queue.try_dequeue("agent-a");
    assert!(
        b1.is_some(),
        "burst permit 1 should be available after threshold"
    );

    let b2 = queue.try_dequeue("agent-a");
    assert!(
        b2.is_some(),
        "burst permit 2 should be available after threshold"
    );

    let b3 = queue.try_dequeue("agent-a");
    assert!(
        b3.is_some(),
        "burst permit 3 should be available after threshold"
    );

    // All burst permits used, should be none left.
    assert!(
        queue.try_dequeue("agent-a").is_none(),
        "no more burst permits"
    );

    assert_eq!(queue.depth("agent-a"), 0);
}

#[test]
fn test_depth_reports_pending_count() {
    let queue = make_queue(3, 100);
    enqueue_n(&queue, "agent-a", 10);

    // Dequeue 3 (max_concurrent).
    let _p1 = queue.try_dequeue("agent-a").unwrap();
    let _p2 = queue.try_dequeue("agent-a").unwrap();
    let _p3 = queue.try_dequeue("agent-a").unwrap();

    // 10 - 3 = 7 pending.
    assert_eq!(queue.depth("agent-a"), 7);
}

#[test]
fn test_per_agent_isolation() {
    let queue = make_queue(2, 100);

    // Fill agent-a to capacity.
    enqueue_n(&queue, "agent-a", 4);
    let _pa1 = queue.try_dequeue("agent-a").unwrap();
    let _pa2 = queue.try_dequeue("agent-a").unwrap();
    // agent-a permits exhausted.
    assert!(queue.try_dequeue("agent-a").is_none());

    // agent-b should still be able to enqueue and dequeue freely.
    enqueue_n(&queue, "agent-b", 2);
    let pb1 = queue.try_dequeue("agent-b");
    assert!(pb1.is_some(), "agent-b should not be blocked by agent-a");
    let pb2 = queue.try_dequeue("agent-b");
    assert!(pb2.is_some(), "agent-b should get its own permits");
}

#[test]
fn test_try_dequeue_empty_returns_none() {
    let queue = make_queue(10, 100);
    assert!(
        queue.try_dequeue("nonexistent-agent").is_none(),
        "try_dequeue on empty/unknown agent should return None"
    );
}
