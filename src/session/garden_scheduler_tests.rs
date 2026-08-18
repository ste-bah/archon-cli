//! What the seam does, driven without waiting fifteen minutes for a timer.
//!
//! `run_one_tick` is exercised directly. The loop above it holds nothing but
//! timing, and a test of a timer would measure tokio rather than this.

use std::sync::Arc;

use archon_learning::garden_proposals::{GardenProposalStatus, list_garden_proposals};
use archon_memory::MemoryTrait;
use archon_memory::garden::GardenConfig;
use archon_memory::types::MemoryType;

use super::{GardenSchedulerSpec, run_one_tick, spawn_garden_scheduler};

/// A store holding memories every pruning rule agrees should go, backdated so
/// staleness has something to measure.
fn doomed_store() -> (Arc<dyn MemoryTrait>, Vec<String>) {
    let graph = archon_memory::MemoryGraph::in_memory().expect("graph");
    let mut ids = Vec::new();
    for i in 0..3 {
        let id = graph
            .store_memory(
                &format!("a fact nobody has looked at in months, number {i}"),
                "forgotten",
                MemoryType::Fact,
                0.1,
                &[],
                "test",
                "",
            )
            .expect("store");
        backdate(&graph, &id, 120);
        ids.push(id);
    }
    (Arc::new(graph) as Arc<dyn MemoryTrait>, ids)
}

fn backdate(graph: &archon_memory::MemoryGraph, id: &str, days: i64) {
    use cozo::{DataValue, ScriptMutability};
    let created_at = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    graph
        .db()
        .run_script(
            "?[id, content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed] :=
                *memories{id, content, title, memory_type, importance, tags, source_type,
                    project_path, updated_at, access_count, last_accessed},
                id = $id, created_at = $created_at
             :put memories { id => content, title, memory_type, importance, tags, source_type,
                project_path, created_at, updated_at, access_count, last_accessed }",
            std::collections::BTreeMap::from([
                ("id".to_string(), DataValue::from(id)),
                (
                    "created_at".to_string(),
                    DataValue::from(created_at.as_str()),
                ),
            ]),
            ScriptMutability::Mutable,
        )
        .expect("backdate");
}

fn learning_db() -> Arc<cozo::DbInstance> {
    let db = Arc::new(
        cozo::DbInstance::new("mem", "", Default::default()).expect("in-memory learning db"),
    );
    archon_learning::schema::ensure_learning_schema(&db).expect("schema");
    db
}

fn enabled_config() -> GardenConfig {
    GardenConfig {
        scheduled_consolidation: true,
        // Zero so a tick is always due; the interval itself has its own test in
        // archon-memory.
        scheduled_interval_hours: 0,
        staleness_days: 30,
        staleness_importance_floor: 0.3,
        ..GardenConfig::default()
    }
}

#[tokio::test]
async fn the_scheduler_does_not_start_by_default() {
    // Whatever else changes, this must not. An upgrade that quietly begins
    // pruning a user's memories on a timer is the failure this whole change is
    // built around avoiding.
    let (memory, _ids) = doomed_store();
    let dir = tempfile::tempdir().expect("tempdir");

    let handle = spawn_garden_scheduler(GardenSchedulerSpec {
        garden: GardenConfig::default(),
        memory,
        data_dir: dir.path().to_path_buf(),
        working_dir: dir.path().to_path_buf(),
        learning_db: None,
    });

    assert!(
        handle.is_none(),
        "scheduled consolidation must not run unless someone switched it on"
    );
}

#[tokio::test]
async fn an_enabled_scheduler_starts() {
    let (memory, _ids) = doomed_store();
    let dir = tempfile::tempdir().expect("tempdir");

    let handle = spawn_garden_scheduler(GardenSchedulerSpec {
        garden: enabled_config(),
        memory,
        data_dir: dir.path().to_path_buf(),
        working_dir: dir.path().to_path_buf(),
        learning_db: None,
    });

    let handle = handle.expect("an enabled scheduler must produce a task");
    handle.abort();
}

#[tokio::test]
async fn a_tick_files_proposals_and_deletes_nothing() {
    let (memory, ids) = doomed_store();
    let dir = tempfile::tempdir().expect("tempdir");
    let db = learning_db();

    run_one_tick(
        &enabled_config(),
        &memory,
        dir.path(),
        dir.path(),
        Some(db.as_ref()),
    )
    .await;

    let pending = list_garden_proposals(&db, GardenProposalStatus::Pending).expect("list pending");
    assert_eq!(pending.len(), 3, "each stale memory must be offered once");
    assert!(
        pending.iter().all(|p| p.proposal_kind
            == archon_learning::garden_proposals::GardenProposalKind::MemoryRetirement),
        "a memory proposed for retirement must not also be proposed as \
         consolidation evidence"
    );
    assert!(
        pending.iter().all(|p| !p.detail.is_empty()),
        "a reviewer needs the evidence, not just the verdict"
    );
    for id in &ids {
        assert!(
            memory.inspect_memory(id).is_ok(),
            "a scheduled pass must not have deleted a memory it proposed retiring"
        );
    }
}

#[tokio::test]
async fn a_proposal_is_never_filed_as_anything_but_pending() {
    // The background job proposes; deciding is a human act. If this ever failed,
    // an approval nobody gave would be sitting in the store waiting for an
    // applier to act on it.
    let (memory, _ids) = doomed_store();
    let dir = tempfile::tempdir().expect("tempdir");
    let db = learning_db();

    run_one_tick(
        &enabled_config(),
        &memory,
        dir.path(),
        dir.path(),
        Some(db.as_ref()),
    )
    .await;

    for status in [
        GardenProposalStatus::Approved,
        GardenProposalStatus::Applied,
        GardenProposalStatus::Rejected,
    ] {
        assert!(
            list_garden_proposals(&db, status).expect("list").is_empty(),
            "the background pass wrote a {} proposal",
            status.as_str()
        );
    }
}

#[tokio::test]
async fn repeated_ticks_do_not_pile_up_duplicate_proposals() {
    // A nightly job re-derives the same candidates from the same untouched
    // store. Without stable proposal ids a reviewer would face one row per
    // night for every memory nobody has touched.
    let (memory, _ids) = doomed_store();
    let dir = tempfile::tempdir().expect("tempdir");
    let db = learning_db();
    let config = enabled_config();
    let data_dir = dir.path().to_path_buf();

    run_one_tick(&config, &memory, &data_dir, dir.path(), Some(db.as_ref())).await;
    run_one_tick(&config, &memory, &data_dir, dir.path(), Some(db.as_ref())).await;
    run_one_tick(&config, &memory, &data_dir, dir.path(), Some(db.as_ref())).await;

    let pending = list_garden_proposals(&db, GardenProposalStatus::Pending).expect("list pending");
    assert_eq!(pending.len(), 3);
}

#[tokio::test]
async fn a_quiet_correction_derived_rule_reaches_the_review_pile_without_being_touched() {
    // The rule half of the loop, end to end: analysis holds no store handle, the
    // seam files the proposal, and the rule itself is untouched until someone
    // approves it.
    use archon_consciousness::rules::{RuleSource, RulesEngine};

    let graph = archon_memory::MemoryGraph::in_memory().expect("graph");
    let rule_id = {
        let engine = RulesEngine::new(&graph);
        let rule = engine
            .add_rule(
                "check constraints before acting",
                RuleSource::CorrectionDerived,
            )
            .expect("add rule");
        rule.id
    };
    // Older than the minimum age, with no corrections and no triggering.
    backdate(&graph, &rule_id, 400);
    let memory: Arc<dyn MemoryTrait> = Arc::new(graph);
    let dir = tempfile::tempdir().expect("tempdir");
    let db = learning_db();

    run_one_tick(
        &enabled_config(),
        &memory,
        dir.path(),
        dir.path(),
        Some(db.as_ref()),
    )
    .await;

    let pending = list_garden_proposals(&db, GardenProposalStatus::Pending).expect("list");
    let rule_proposal = pending
        .iter()
        .find(|proposal| {
            proposal.proposal_kind
                == archon_learning::garden_proposals::GardenProposalKind::RuleRetirement
        })
        .expect("a quiet rule should be proposed for retirement");
    assert_eq!(rule_proposal.subject_id, rule_id);
    assert!(
        rule_proposal.detail.contains("no supporting correction"),
        "the evidence must come from correction records: {}",
        rule_proposal.detail
    );

    let untouched = memory.inspect_memory(&rule_id).expect("rule still there");
    assert!(
        !archon_memory::is_retired(&untouched.tags),
        "raising a proposal must not retire the rule"
    );
    assert_eq!(untouched.content, "check constraints before acting");
}

#[tokio::test]
async fn a_tick_with_no_governed_store_still_refuses_to_delete() {
    // Losing the review pile must never be an excuse to prune directly. A pass
    // with nowhere to file its proposals does less, not more.
    let (memory, ids) = doomed_store();
    let dir = tempfile::tempdir().expect("tempdir");

    run_one_tick(&enabled_config(), &memory, dir.path(), dir.path(), None).await;

    for id in &ids {
        assert!(
            memory.inspect_memory(id).is_ok(),
            "no governed store must not become permission to delete"
        );
    }
}

#[tokio::test]
async fn a_tick_declined_by_the_run_lock_changes_nothing() {
    let (memory, ids) = doomed_store();
    let dir = tempfile::tempdir().expect("tempdir");
    let db = learning_db();

    let lock_path = archon_memory::garden::run_lock_path(dir.path());
    let foreign = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .expect("open lock file");
    let mut foreign_lock = fd_lock::RwLock::new(foreign);
    let _held = foreign_lock.try_write().expect("foreign holder takes it");

    run_one_tick(
        &enabled_config(),
        &memory,
        dir.path(),
        dir.path(),
        Some(db.as_ref()),
    )
    .await;

    assert!(
        list_garden_proposals(&db, GardenProposalStatus::Pending)
            .expect("list")
            .is_empty(),
        "a declined tick must not have read the store, let alone proposed anything"
    );
    for id in &ids {
        assert!(memory.inspect_memory(id).is_ok());
    }
}
