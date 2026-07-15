use std::sync::{Arc, Barrier, mpsc};
use std::thread;

use super::*;

#[test]
fn concurrent_explicit_id_outcomes_report_one_creator() {
    let graph = Arc::new(make_graph());
    let barrier = Arc::new(Barrier::new(3));
    let (tx, rx) = mpsc::channel();

    for _ in 0..2 {
        let graph = Arc::clone(&graph);
        let barrier = Arc::clone(&barrier);
        let tx = tx.clone();
        thread::spawn(move || {
            barrier.wait();
            tx.send(graph.store_memory_with_id_outcome(
                "rule:correction:created-outcome",
                "Avoid concurrent mutation",
                "",
                MemoryType::Rule,
                50.0,
                &["source:correction-derived".into(), "trend:stable".into()],
                "rules_engine",
                "",
            ))
            .expect("send result");
        });
    }

    barrier.wait();
    let outcomes: Vec<_> = (0..2)
        .map(|_| rx.recv().expect("receive result").expect("store memory"))
        .collect();
    assert_eq!(outcomes.iter().filter(|outcome| outcome.created).count(), 1);
    assert_eq!(
        outcomes.iter().filter(|outcome| !outcome.created).count(),
        1
    );
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome.memory.id == "rule:correction:created-outcome")
    );
}

#[test]
fn concurrent_explicit_id_creates_converge_to_one_authoritative_memory() {
    let graph = Arc::new(make_graph());
    let barrier = Arc::new(Barrier::new(3));
    let (tx, rx) = mpsc::channel();

    for _ in 0..2 {
        let graph = Arc::clone(&graph);
        let barrier = Arc::clone(&barrier);
        let tx = tx.clone();
        thread::spawn(move || {
            barrier.wait();
            tx.send(graph.store_memory_with_id(
                "rule:correction:convergent",
                "Avoid: concurrent mutation",
                "",
                MemoryType::Rule,
                50.0,
                &["source:correction-derived".into(), "trend:stable".into()],
                "rules_engine",
                "",
            ))
            .expect("send result");
        });
    }

    barrier.wait();
    for _ in 0..2 {
        assert_eq!(
            rx.recv().expect("receive result").expect("store memory").id,
            "rule:correction:convergent"
        );
    }
    assert_eq!(graph.memory_count().expect("count memories"), 1);
}
