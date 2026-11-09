use super::*;
#[test]
fn missing_requirement_contradiction_is_once_per_task_per_process() {
    let (mut universe, fixture) = contract();
    let task = universe
        .tasks
        .iter_mut()
        .find(|t| t.canonical_task_id == "TASK-TDL-001")
        .unwrap();
    task.requirement_states
        .insert("item".into(), "missing".into());
    let contract = LifecycleContract {
        task_universe: &universe,
        target_repository_root: Some("/repo"),
    };
    let (first, ids) = reclassify_inventory_contradicted_noops(
        &contract,
        &fixture["inventory"],
        &BTreeSet::new(),
        &BTreeSet::new(),
    );
    assert!(ids.contains("TASK-TDL-001"));
    assert!(
        support::array(first.get("items"))
            .iter()
            .any(|i| i["noop_reclassification"]["source"] == "missing_requirement")
    );
    let (second, again) = reclassify_inventory_contradicted_noops(
        &contract,
        &fixture["inventory"],
        &ids,
        &BTreeSet::new(),
    );
    assert!(again.is_empty());
    assert_eq!(second, fixture["inventory"]);
    let (_, completed) = reclassify_inventory_contradicted_noops(
        &contract,
        &fixture["inventory"],
        &BTreeSet::new(),
        &ids,
    );
    assert!(completed.is_empty());
}
