use super::*;
use crate::task_universe::WorkflowV2TaskUniverse;

fn task(id: &str, deps: &[&str]) -> crate::task_universe::WorkflowV2TaskUniverseTask {
    crate::task_universe::WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        aliases: Vec::new(),
        source_path: format!("tasks/{id}.md"),
        dependency_ids: deps.iter().map(|d| (*d).to_string()).collect(),
        title: None,
        artifact_requirements: Vec::new(),
        ..Default::default()
    }
}

fn typed_universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-EX-001", &[]),
            task("TASK-EX-010", &["TASK-EX-001"]),
            task("TASK-EX-020", &["TASK-EX-010"]),
        ],
    }
}

fn contract_for(universe: &WorkflowV2TaskUniverse) -> LifecycleContract<'_> {
    LifecycleContract {
        task_universe: universe,
        target_repository_root: Some("/repo"),
    }
}

fn grouped() -> Value {
    serde_json::json!({
        "item_id": "noop-group-001-010-020",
        "canonical_task_ids": ["TASK-EX-001", "TASK-EX-010", "TASK-EX-020"],
        "work_type": "verified_noop",
    })
}

fn part(id: &str, task: &str) -> Value {
    serde_json::json!({
        "item_id": id,
        "canonical_task_ids": [task],
        "work_type": "implementation",
    })
}

/// The live shape: one item grouping a task with its own prerequisite, and a
/// repair that splits it into one item per task. Without recognising the
/// split, every part merges back into the group and the grouping survives
/// forever.
#[test]
fn a_covering_split_supersedes_the_grouped_item() {
    let universe = typed_universe();
    let contract = contract_for(&universe);
    let parts = vec![
        part("impl-001", "TASK-EX-001"),
        part("impl-010", "TASK-EX-010"),
        part("impl-020", "TASK-EX-020"),
    ];

    let superseded = grouped_items_superseded_by_splits(&contract, &[grouped()], &parts);

    assert!(
        superseded.contains("item:noop-group-001-010-020"),
        "{superseded:?}"
    );
}

/// The tombstone guarantee: a split that leaves a task with no home would shed
/// scheduled work, so it is refused and the group stands.
#[test]
fn a_split_that_drops_a_task_is_refused() {
    let universe = typed_universe();
    let contract = contract_for(&universe);
    // TASK-EX-020 has no part.
    let parts = vec![
        part("impl-001", "TASK-EX-001"),
        part("impl-010", "TASK-EX-010"),
    ];

    let superseded = grouped_items_superseded_by_splits(&contract, &[grouped()], &parts);

    assert!(
        superseded.is_empty(),
        "incomplete split must not drop the group"
    );
}

/// One repair item is a correction, not a split — the existing merge path
/// handles it and must keep handling it.
#[test]
fn a_single_replacement_item_is_not_a_split() {
    let universe = typed_universe();
    let contract = contract_for(&universe);
    let parts = vec![part("impl-001", "TASK-EX-001")];

    let superseded = grouped_items_superseded_by_splits(&contract, &[grouped()], &parts);

    assert!(superseded.is_empty());
}

/// An item covering one task cannot be split, however many repair items
/// mention it.
#[test]
fn a_single_task_item_is_never_superseded() {
    let universe = typed_universe();
    let contract = contract_for(&universe);
    let single = serde_json::json!({
        "item_id": "impl-solo",
        "canonical_task_ids": ["TASK-EX-001"],
        "work_type": "implementation",
    });
    let parts = vec![part("a", "TASK-EX-001"), part("b", "TASK-EX-001")];

    let superseded = grouped_items_superseded_by_splits(&contract, &[single], &parts);

    assert!(superseded.is_empty());
}

/// A repair item carrying the grouped item's own id is a correction of that
/// item, not one of its parts.
#[test]
fn a_repair_reusing_the_group_id_is_not_a_part() {
    let universe = typed_universe();
    let contract = contract_for(&universe);
    let mut self_edit = part("noop-group-001-010-020", "TASK-EX-001");
    self_edit["canonical_task_ids"] = serde_json::json!(["TASK-EX-001"]);
    let parts = vec![self_edit, part("impl-010", "TASK-EX-010")];

    let superseded = grouped_items_superseded_by_splits(&contract, &[grouped()], &parts);

    assert!(
        superseded.is_empty(),
        "only one genuine part; not a covering split"
    );
}

/// The group's aliases must be cleared, or a split part matches the very
/// group it replaced and folds back in.
#[test]
fn superseding_clears_every_alias_the_group_held() {
    let universe = typed_universe();
    let contract = contract_for(&universe);
    let superseded = BTreeSet::from(["item:noop-group-001-010-020".to_string()]);

    let aliases = superseded_aliases(&contract, &[grouped()], &superseded);

    for key in [
        "item:noop-group-001-010-020",
        "task:TASK-EX-001",
        "task:TASK-EX-010",
        "task:TASK-EX-020",
    ] {
        assert!(aliases.contains_key(key), "missing {key}: {aliases:?}");
    }
}
