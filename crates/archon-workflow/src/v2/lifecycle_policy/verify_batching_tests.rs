use super::*;

fn cargo_item(id: &str, command: &str) -> Value {
    serde_json::json!({
        "item_id": id,
        "canonical_task_ids": ["TASK-EX-020"],
        "source_item_id": "TASK-EX-020",
        "focused_verification": command,
        "expected_evidence": format!("{command} passes"),
        "required_tools": ["cargo"],
        "write_coordination_scope": { "declared_target_files": [] }
    })
}

fn inspection_item(id: &str) -> Value {
    serde_json::json!({
        "item_id": id,
        "canonical_task_ids": ["TASK-EX-020"],
        "source_item_id": "TASK-EX-020",
        "focused_verification": "Verify all deliverable files exist and are non-empty",
        "required_tools": ["bash"]
    })
}

/// The live wave shape: twelve one-test cargo items and file inspections.
/// The cargo items become one branch; inspections pass through untouched.
#[test]
fn same_task_cargo_items_merge_into_one() {
    let items = vec![
        inspection_item("vit-file-existence"),
        cargo_item("vit-test-a", "cargo test -p x a"),
        cargo_item("vit-test-b", "cargo test -p x b"),
        cargo_item("vit-clippy", "cargo clippy --workspace"),
    ];

    let batched = batch_cargo_verification_items(items);

    assert_eq!(batched.len(), 2);
    assert_eq!(batched[0]["item_id"], "vit-file-existence");
    let batch = &batched[1];
    assert_eq!(batch["item_id"], "vit-test-a-cargo-batch");
    assert_eq!(
        batch["batched_from_item_ids"],
        serde_json::json!(["vit-test-a", "vit-test-b", "vit-clippy"])
    );
    let commands = batch["focused_verification"].as_array().unwrap();
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0], "cargo test -p x a");
    assert_eq!(commands[2], "cargo clippy --workspace");
    // Per-command evidence expectations travel with their commands.
    assert_eq!(batch["expected_evidence"].as_array().unwrap().len(), 3);
}

/// Items for different tasks never share a branch: a batch's verdict must map
/// to exactly one task's coverage.
#[test]
fn different_tasks_do_not_merge() {
    let mut other = cargo_item("vit-other", "cargo test -p y c");
    other["canonical_task_ids"] = serde_json::json!(["TASK-EX-030"]);
    other["source_item_id"] = serde_json::json!("TASK-EX-030");
    let items = vec![
        cargo_item("vit-test-a", "cargo test -p x a"),
        cargo_item("vit-test-b", "cargo test -p x b"),
        other,
    ];

    let batched = batch_cargo_verification_items(items);

    assert_eq!(batched.len(), 2);
    assert_eq!(batched[1]["item_id"], "vit-other");
    assert_eq!(batched[0]["item_id"], "vit-test-a-cargo-batch");
}

/// A lone cargo item is left exactly as authored — no rename, no wrapper.
#[test]
fn a_single_cargo_item_is_not_rewritten() {
    let items = vec![cargo_item("vit-only", "cargo test -p x a")];

    let batched = batch_cargo_verification_items(items);

    assert_eq!(batched.len(), 1);
    assert_eq!(batched[0]["item_id"], "vit-only");
    assert!(batched[0].get("batched_from_item_ids").is_none());
}

/// An item that declares write targets is more than a command check and stays
/// out of every batch.
#[test]
fn a_write_scoped_item_never_merges() {
    let mut write_scoped = cargo_item("vit-write", "cargo test -p x w");
    write_scoped["write_coordination_scope"] =
        serde_json::json!({ "declared_target_files": ["src/a.rs"] });
    let items = vec![
        cargo_item("vit-test-a", "cargo test -p x a"),
        write_scoped,
        cargo_item("vit-test-b", "cargo test -p x b"),
    ];

    let batched = batch_cargo_verification_items(items);

    assert_eq!(batched.len(), 2);
    assert_eq!(batched[0]["item_id"], "vit-write");
    assert_eq!(batched[1]["item_id"], "vit-test-a-cargo-batch");
}

/// Oversized batches chunk rather than concentrating dozens of checks in one
/// agent.
#[test]
fn batches_chunk_at_the_cap() {
    let items: Vec<Value> = (0..12)
        .map(|i| cargo_item(&format!("vit-test-{i}"), &format!("cargo test -p x t{i}")))
        .collect();

    let batched = batch_cargo_verification_items(items);

    assert_eq!(batched.len(), 2);
    assert_eq!(
        batched[0]["batched_from_item_ids"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    assert_eq!(
        batched[1]["batched_from_item_ids"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

/// Union fields dedupe: twelve items each requiring "cargo" produce one
/// required_tools entry.
#[test]
fn union_fields_are_deduped() {
    let items = vec![
        cargo_item("vit-test-a", "cargo test -p x a"),
        cargo_item("vit-test-b", "cargo test -p x b"),
    ];

    let batched = batch_cargo_verification_items(items);

    assert_eq!(batched[0]["required_tools"], serde_json::json!(["cargo"]));
}

/// The shape observed live: one cargo item per task, one shared source item.
/// Keying the batch on the task made every key unique and nothing ever merged.
#[test]
fn one_cargo_item_per_task_still_batches() {
    let items = (0..3)
        .map(|index| {
            serde_json::json!({
                "item_id": format!("verification-tdl-0{index}0-cargo-check"),
                "canonical_task_ids": [format!("TASK-TDL-0{index}0")],
                "source_item_id": "wire-tasks-universe",
                "focused_verification": [format!("cargo check -p crate-{index}")],
            })
        })
        .collect();

    let batched = super::batch_cargo_verification_items(items);

    assert_eq!(batched.len(), 1, "{batched:#?}");
}

/// A merged branch spans tasks, so it must declare every one it answers for.
#[test]
fn a_cross_task_batch_declares_every_task_it_covers() {
    let items = vec![
        serde_json::json!({
            "item_id": "a",
            "canonical_task_ids": ["TASK-A"],
            "source_item_id": "shared",
            "focused_verification": ["cargo check -p a"],
        }),
        serde_json::json!({
            "item_id": "b",
            "canonical_task_ids": ["TASK-B"],
            "source_item_id": "shared",
            "focused_verification": ["cargo check -p b"],
        }),
    ];

    let batched = super::batch_cargo_verification_items(items);

    let tasks = &batched[0]["canonical_task_ids"];
    assert_eq!(tasks[0], "TASK-A");
    assert_eq!(tasks[1], "TASK-B");
    assert_eq!(batched[0]["batched_item_provenance"][1]["item_id"], "b");
}

/// Items from different source items stay apart.
#[test]
fn different_source_items_do_not_merge() {
    let items = vec![
        serde_json::json!({
            "item_id": "a",
            "canonical_task_ids": ["TASK-A"],
            "source_item_id": "first",
            "focused_verification": ["cargo check -p a"],
        }),
        serde_json::json!({
            "item_id": "b",
            "canonical_task_ids": ["TASK-B"],
            "source_item_id": "second",
            "focused_verification": ["cargo check -p b"],
        }),
    ];

    assert_eq!(super::batch_cargo_verification_items(items).len(), 2);
}
