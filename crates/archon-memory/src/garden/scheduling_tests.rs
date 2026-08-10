//! What an unattended pass may and may not do, pinned against a real store.
//!
//! The assertions that matter here are negative ones: that a memory is STILL
//! THERE after a scheduled pass decided it was stale. A positive test — "the
//! candidate was reported" — would pass just as happily on a build that
//! reported the candidate and deleted the row.

use std::collections::BTreeMap;

use cozo::{DataValue, ScriptMutability};

use super::{GardenRunPolicy, ScheduledRun, run_scheduled_consolidation, should_run_scheduled};
use crate::MemoryGraph;
use crate::access::MemoryTrait;
use crate::garden::{GardenConfig, RetirementReason, consolidate_with_policy, run_lock_path};
use crate::types::MemoryType;

/// Backdate a memory so staleness and decay have something to bill.
fn age_memory(graph: &MemoryGraph, id: &str, days: i64) {
    let created_at = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    graph
        .db
        .run_script(
            "?[id, content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed] :=
                *memories{id, content, title, memory_type, importance, tags, source_type,
                    project_path, updated_at, access_count, last_accessed},
                id = $id, created_at = $created_at
             :put memories { id => content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed }",
            BTreeMap::from([
                ("id".to_string(), DataValue::from(id)),
                (
                    "created_at".to_string(),
                    DataValue::from(created_at.as_str()),
                ),
            ]),
            ScriptMutability::Mutable,
        )
        .expect("age memory");
}

/// A store holding one memory that every prune rule agrees should go.
fn store_with_one_doomed_memory() -> (MemoryGraph, String) {
    let graph = MemoryGraph::in_memory().expect("create graph");
    let id = graph
        .store_memory(
            "a fact nobody has looked at in months",
            "doomed",
            MemoryType::Fact,
            0.1,
            &[],
            "test",
            "",
        )
        .expect("store fact");
    age_memory(&graph, &id, 90);
    (graph, id)
}

fn scheduled_config() -> GardenConfig {
    GardenConfig {
        scheduled_consolidation: true,
        // Zero so a test can run two passes back to back without waiting a day.
        scheduled_interval_hours: 0,
        staleness_days: 30,
        staleness_importance_floor: 0.3,
        ..GardenConfig::default()
    }
}

#[test]
fn the_scheduler_is_off_by_default() {
    // The single most important assertion about this feature. Unattended pruning
    // of a user's memories is not something to opt people into.
    assert!(
        !should_run_scheduled(&GardenConfig::default()),
        "scheduled consolidation must be off unless someone switched it on"
    );
}

#[test]
fn a_config_written_before_the_scheduler_existed_reads_as_off() {
    // An upgrade must not silently start pruning. Every new field has to default
    // to the inert value when it is simply absent from an existing config file.
    let legacy = r#"{
        "auto_consolidate": true,
        "min_hours_between_runs": 24,
        "dedup_similarity_threshold": 0.92,
        "staleness_days": 30,
        "staleness_importance_floor": 0.3,
        "importance_decay_per_day": 0.01,
        "max_memories": 5000,
        "briefing_limit": 15
    }"#;

    let config: GardenConfig = serde_json::from_str(legacy).expect("legacy config parses");

    assert!(!config.scheduled_consolidation);
    assert!(!should_run_scheduled(&config));
}

#[test]
fn a_scheduled_pass_does_not_delete_a_stale_memory() {
    let (graph, id) = store_with_one_doomed_memory();
    let dir = tempfile::tempdir().expect("tempdir");

    let run = run_scheduled_consolidation(&graph, &scheduled_config(), dir.path(), "run-1")
        .expect("scheduled pass runs");

    let report = run.report().expect("a pass should have run");
    assert_eq!(
        report.stale_pruned, 0,
        "an unattended pass must not delete anything"
    );
    assert!(
        graph.inspect_memory(&id).is_ok(),
        "the memory a scheduled pass judged stale must still be there"
    );
    assert_eq!(report.retirement_candidates.len(), 1);
    let candidate = &report.retirement_candidates[0];
    assert_eq!(candidate.memory_id, id);
    assert!(matches!(candidate.reason, RetirementReason::Stale { .. }));
}

#[test]
fn the_manual_pass_still_deletes_what_it_always_did() {
    // The restriction is on the unattended path only. If this ever fails, the
    // safety work has quietly turned `/garden` into a no-op.
    let (graph, id) = store_with_one_doomed_memory();

    let report = consolidate_with_policy(
        &graph,
        &GardenConfig::default(),
        "manual",
        GardenRunPolicy::interactive(),
    )
    .expect("manual pass runs");

    assert_eq!(report.stale_pruned, 1);
    assert!(
        report.retirement_candidates.is_empty(),
        "a pass that deletes has nothing to propose"
    );
    assert!(
        graph.inspect_memory(&id).is_err(),
        "the manual pass prunes, as it always has"
    );
}

#[test]
fn a_scheduled_pass_proposes_overflow_candidates_without_deleting() {
    let graph = MemoryGraph::in_memory().expect("create graph");
    let mut ids = Vec::new();
    for i in 0..6 {
        ids.push(
            graph
                .store_memory(
                    &format!("distinct memory number {i}"),
                    "keep",
                    MemoryType::Fact,
                    0.5 + f64::from(i),
                    &[],
                    "test",
                    "",
                )
                .expect("store"),
        );
    }
    let config = GardenConfig {
        // Well under the row count, so overflow certainly fires. Staleness is
        // pushed out of reach so this test only measures the overflow rule.
        max_memories: 2,
        staleness_days: 3650,
        ..scheduled_config()
    };
    let dir = tempfile::tempdir().expect("tempdir");

    let report = run_scheduled_consolidation(&graph, &config, dir.path(), "run-overflow")
        .expect("scheduled pass runs")
        .report()
        .expect("a pass should have run");

    assert_eq!(report.overflow_pruned, 0);
    assert!(
        !report.retirement_candidates.is_empty(),
        "an over-cap store must produce candidates, or the pass is inert"
    );
    assert!(
        report
            .retirement_candidates
            .iter()
            .all(|c| matches!(c.reason, RetirementReason::Overflow { .. })),
        "with staleness out of reach, every candidate must be an overflow one"
    );
    for id in &ids {
        assert!(
            graph.inspect_memory(id).is_ok(),
            "no memory may be destroyed by an unattended overflow pass"
        );
    }
}

#[test]
fn a_scheduled_pass_declines_while_another_holds_the_run_lock() {
    let (graph, id) = store_with_one_doomed_memory();
    let dir = tempfile::tempdir().expect("tempdir");
    let lock_path = run_lock_path(dir.path());

    // A foreign holder, exactly as a second Archon process would present.
    let foreign = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("open lock file");
    let mut foreign_lock = fd_lock::RwLock::new(foreign);
    let _held = foreign_lock.try_write().expect("foreign holder takes it");

    let run = run_scheduled_consolidation(&graph, &scheduled_config(), dir.path(), "run-blocked")
        .expect("contention is not an error");

    assert!(matches!(run, ScheduledRun::Declined));
    assert!(
        graph.inspect_memory(&id).is_ok(),
        "a declined pass must not have touched the store"
    );
}

#[test]
fn a_scheduled_pass_respects_the_interval() {
    let graph = MemoryGraph::in_memory().expect("create graph");
    let config = GardenConfig {
        scheduled_interval_hours: 24,
        ..scheduled_config()
    };
    let dir = tempfile::tempdir().expect("tempdir");

    let first =
        run_scheduled_consolidation(&graph, &config, dir.path(), "run-1").expect("first pass runs");
    assert!(matches!(first, ScheduledRun::Ran(_)));

    let second = run_scheduled_consolidation(&graph, &config, dir.path(), "run-2")
        .expect("second attempt is not an error");
    assert!(
        matches!(second, ScheduledRun::TooRecent),
        "a tick inside the interval must not consolidate again"
    );
}

#[test]
fn the_work_budget_stops_a_scheduled_pass_and_says_so() {
    let graph = MemoryGraph::in_memory().expect("create graph");
    for i in 0..10 {
        let id = graph
            .store_memory(
                &format!("aging memory number {i}"),
                "age",
                MemoryType::Fact,
                50.0,
                &[],
                "test",
                "",
            )
            .expect("store");
        age_memory(&graph, &id, 5);
    }
    let config = GardenConfig {
        // Decay is the only phase with work to do here, so the ceiling lands on
        // a countable number of units.
        scheduled_max_reversible_ops: 3,
        staleness_days: 3650,
        max_memories: 5000,
        importance_decay_per_day: 1.0,
        ..scheduled_config()
    };
    let dir = tempfile::tempdir().expect("tempdir");

    let report = run_scheduled_consolidation(&graph, &config, dir.path(), "run-budget")
        .expect("scheduled pass runs")
        .report()
        .expect("a pass should have run");

    assert_eq!(
        report.importance_decayed, 3,
        "the pass must stop at its ceiling, not overrun it"
    );
    assert!(
        report.budget_exhausted,
        "stopping short must be reported, or an over-budget store looks healthy"
    );
}

#[test]
fn a_budget_stopped_pass_leaves_a_store_the_next_pass_can_proceed_from() {
    // The interruption property, exercised rather than argued: run a pass that
    // is guaranteed to stop early, then run another, and check the second one
    // picks up rather than redoing or corrupting the first one's work.
    let graph = MemoryGraph::in_memory().expect("create graph");
    let mut ids = Vec::new();
    for i in 0..6 {
        let id = graph
            .store_memory(
                &format!("aging memory number {i}"),
                "age",
                MemoryType::Fact,
                50.0,
                &[],
                "test",
                "",
            )
            .expect("store");
        age_memory(&graph, &id, 5);
        ids.push(id);
    }
    let config = GardenConfig {
        scheduled_max_reversible_ops: 2,
        staleness_days: 3650,
        importance_decay_per_day: 1.0,
        ..scheduled_config()
    };
    let dir = tempfile::tempdir().expect("tempdir");

    let first = run_scheduled_consolidation(&graph, &config, dir.path(), "run-a")
        .expect("first pass")
        .report()
        .expect("ran");
    assert_eq!(first.importance_decayed, 2);
    assert!(first.budget_exhausted);

    // Every row is intact and readable -- nothing was left half-written.
    for id in &ids {
        let memory = graph.inspect_memory(id).expect("row survives the stop");
        assert!(
            memory.importance == 50.0 || memory.importance == 45.0,
            "importance must be either untouched or fully decayed, never partial: {}",
            memory.importance
        );
    }

    let second = run_scheduled_consolidation(&graph, &config, dir.path(), "run-b")
        .expect("second pass")
        .report()
        .expect("ran");
    assert!(
        second.importance_decayed <= 2,
        "the second pass is bounded by the same ceiling"
    );
    assert_eq!(
        graph.memory_count().expect("count"),
        // Six memories plus the garden's own run-timestamp row.
        7,
        "no row may be lost across an interrupted pass and its successor"
    );
}

#[path = "scheduling_tests/consolidation.rs"]
mod consolidation;
