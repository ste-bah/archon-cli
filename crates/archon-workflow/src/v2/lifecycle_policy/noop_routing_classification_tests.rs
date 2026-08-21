use super::*;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

fn two_task_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: ["TASK-TDL-020", "TASK-TDL-030"]
            .into_iter()
            .map(|id| WorkflowV2TaskUniverseTask {
                canonical_task_id: id.to_string(),
                source_path: format!("tasks/{id}.md"),
                acceptance_criteria: vec!["Criterion one.".to_string()],
                ..Default::default()
            })
            .collect(),
    }
}

fn noop_item(task_id: &str) -> Value {
    serde_json::json!({
        "item_id": format!("noop-{task_id}"),
        "work_type": "verified_noop",
        "canonical_task_ids": [task_id],
        "dependency_ids": [],
        "acceptance_criteria": ["Criterion one."],
        "noop_proof": "already implemented",
        "noop_proof_refs": ["evidence/proof.md"],
    })
}

/// The live halt. The host rejected the wave with
/// `source_data[0].acceptance_criteria is missing or empty` — a complaint about
/// input shape, carrying no canonical task id. With more than one noop item the
/// old single-item fallback did not apply, nothing could be reclassified, and
/// the run blocked. Twelve identical iterations, while the paired evidence
/// repair had already found three real TASK-TDL-020 failures.
#[test]
fn an_unattributed_refutation_reclassifies_a_multi_item_wave() {
    let universe = two_task_universe();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let items = vec![noop_item("TASK-TDL-020"), noop_item("TASK-TDL-030")];
    // A host shape failure: residual gaps, but no canonical task id anywhere.
    let failed = vec![serde_json::json!({
        "status": "needs_review",
        "summary": "noop proof source_data[0].acceptance_criteria is missing or empty",
        "residual_gaps": [{
            "id": "dynamic_wave_source_metadata_noop-proof-verification-1",
            "description": "noop proof source_data[0].acceptance_criteria is missing or empty",
        }],
    })];
    let mut reclassified = BTreeSet::new();

    let route = route_refuted_noops(
        &contract,
        &items,
        &BTreeSet::new(),
        &failed,
        &BTreeSet::new(),
        &mut reclassified,
    );

    match route {
        NoopProofExhaustionRoute::ScheduleImplementation(scheduled) => {
            assert_eq!(scheduled.len(), 2, "both unproven noops become work");
        }
        NoopProofExhaustionRoute::Block => {
            panic!("an unproven noop must become implementation work, not halt the run")
        }
    }
}

/// A transport failure is not evidence about the task, so it must not convert
/// a no-op into work on the strength of an infrastructure blip.
#[test]
fn a_transport_failure_still_blocks_rather_than_reclassifying() {
    let universe = two_task_universe();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let items = vec![noop_item("TASK-TDL-020"), noop_item("TASK-TDL-030")];
    let failed = vec![serde_json::json!({
        "status": "failed",
        "failure_class": "transport",
        "residual_gaps": [{ "id": "transport", "description": "stream idle timeout" }],
    })];
    let mut reclassified = BTreeSet::new();

    assert_eq!(
        route_refuted_noops(
            &contract,
            &items,
            &BTreeSet::new(),
            &failed,
            &BTreeSet::new(),
            &mut reclassified,
        ),
        NoopProofExhaustionRoute::Block
    );
}

/// When the refutation does name tasks, only those are reclassified — the
/// fallback must not widen an attributed refutation into the whole wave.
#[test]
fn an_attributed_refutation_reclassifies_only_the_named_task() {
    let universe = two_task_universe();
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let items = vec![noop_item("TASK-TDL-020"), noop_item("TASK-TDL-030")];
    let failed = vec![serde_json::json!({
        "status": "needs_review",
        "canonical_task_ids": ["TASK-TDL-020"],
        "residual_gaps": [{ "id": "gap", "description": "not actually a no-op" }],
    })];
    let mut reclassified = BTreeSet::new();

    match route_refuted_noops(
        &contract,
        &items,
        &BTreeSet::new(),
        &failed,
        &BTreeSet::new(),
        &mut reclassified,
    ) {
        NoopProofExhaustionRoute::ScheduleImplementation(scheduled) => {
            assert_eq!(scheduled.len(), 1);
            assert!(reclassified.contains("TASK-TDL-020"));
            assert!(!reclassified.contains("TASK-TDL-030"));
        }
        NoopProofExhaustionRoute::Block => panic!("the named task must be scheduled"),
    }
}
