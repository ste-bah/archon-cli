use super::*;

const PRD: &str = "# PRD\n\n## Requirements\n\n- REQ-ONE-001: the tool reads a file.\n- REQ-ONE-002: the tool writes a file.\n";

fn run_store() -> (tempfile::TempDir, WorkflowStore, &'static str) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run_id = "wf-inventory";
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    std::fs::write(store.events_path(run_id), "").unwrap();
    (temp, store, run_id)
}

fn texts(ids: &[&str]) -> BTreeMap<String, String> {
    ids.iter()
        .map(|id| (id.to_string(), format!("text of {id}")))
        .collect()
}

#[test]
fn fixed_resume_wires_inventory_drift_into_its_output() {
    let source = include_str!("workflow_decompose_resume.rs");
    assert!(
        source.contains("workflow_decompose_requirement_inventory::check_on_resume("),
        "fixed resume must run the requirement inventory check"
    );
    assert!(
        source.contains(".chain(inventory)"),
        "fixed resume must emit inventory drift lines"
    );
}

#[test]
fn the_launch_freezes_the_ids_its_parser_derives() {
    let (_temp, store, run_id) = run_store();
    record_launch(&store, run_id, "prd-digest", PRD).unwrap();
    let record: RequirementInventoryV1 =
        serde_json::from_slice(&std::fs::read(store.run_dir(run_id).join(INVENTORY_PATH)).unwrap())
            .unwrap();
    let derived = archon_workflow::v2::acceptance_stage::coverage::prd_requirement_texts(PRD);
    assert!(
        !derived.is_empty(),
        "the fixture PRD must name requirement ids"
    );
    assert_eq!(record.ids, derived.keys().cloned().collect::<BTreeSet<_>>());
    assert_eq!(record.recorded_at, "launch");
    assert_eq!(record.digest, inventory_digest(&record.ids));
    // The same parse on resume reports nothing.
    assert_eq!(
        compare(&store, run_id, "prd-digest", &derived, false).unwrap(),
        None
    );
}

#[test]
fn a_resume_names_each_id_a_changed_parser_added_or_removed() {
    let (temp, store, run_id) = run_store();
    let frozen = texts(&["REQ-A-001", "REQ-A-002"]);
    store
        .write_run_json(
            run_id,
            INVENTORY_PATH,
            &RequirementInventoryV1::of("d", &frozen, "launch"),
        )
        .unwrap();
    let now = texts(&["REQ-A-001", "REQ-A-003"]);
    let change = compare(&store, run_id, "d", &now, true).unwrap().unwrap();
    assert_eq!(change.added, vec!["REQ-A-003".to_string()]);
    assert_eq!(change.removed, vec!["REQ-A-002".to_string()]);

    let log = temp.path().join(".decompose.log");
    let lines = check_on_resume(&store, run_id, &log, "d", &now).unwrap();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("requirement_inventory_changed added=REQ-A-003 removed=REQ-A-002 "),
        "{}",
        lines[0]
    );
    assert!(std::fs::read_to_string(&log).unwrap().contains(&lines[0]));
    let events = std::fs::read_to_string(store.events_path(run_id)).unwrap();
    let event: serde_json::Value = serde_json::from_str(events.lines().last().unwrap()).unwrap();
    assert_eq!(event["kind"], "binary_revision_drift", "{event}");
    assert_eq!(event["detail"]["event"], "requirement_inventory_changed");
    assert_eq!(event["detail"]["added"], serde_json::json!(["REQ-A-003"]));
    assert_eq!(event["detail"]["removed"], serde_json::json!(["REQ-A-002"]));
    // The frozen record is kept, so the next resume names the change again.
    let kept: RequirementInventoryV1 =
        serde_json::from_slice(&std::fs::read(store.run_dir(run_id).join(INVENTORY_PATH)).unwrap())
            .unwrap();
    assert_eq!(kept.ids, frozen.keys().cloned().collect::<BTreeSet<_>>());
}

#[test]
fn an_added_id_alone_is_named_even_when_every_text_is_unchanged() {
    let (_temp, store, run_id) = run_store();
    let frozen = texts(&["REQ-A-001"]);
    store
        .write_run_json(
            run_id,
            INVENTORY_PATH,
            &RequirementInventoryV1::of("d", &frozen, "launch"),
        )
        .unwrap();
    let change = compare(
        &store,
        run_id,
        "d",
        &texts(&["REQ-A-001", "REQ-A-002"]),
        true,
    )
    .unwrap()
    .unwrap();
    assert_eq!(change.added, vec!["REQ-A-002".to_string()]);
    assert!(change.removed.is_empty());
}

#[test]
fn a_run_launched_before_the_record_takes_its_first_resume_as_baseline() {
    let (_temp, store, run_id) = run_store();
    let path = store.run_dir(run_id).join(INVENTORY_PATH);
    let now = texts(&["REQ-A-001"]);
    assert_eq!(compare(&store, run_id, "d", &now, true).unwrap(), None);
    assert!(!path.exists(), "a dry run writes no baseline");
    assert_eq!(compare(&store, run_id, "d", &now, false).unwrap(), None);
    let record: RequirementInventoryV1 =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(record.recorded_at, "first-resume");
    assert_eq!(
        compare(
            &store,
            run_id,
            "d",
            &texts(&["REQ-A-001", "REQ-B-001"]),
            true
        )
        .unwrap()
        .unwrap()
        .added,
        vec!["REQ-B-001".to_string()]
    );
}

#[test]
fn an_unreadable_record_is_an_error_not_a_silent_baseline() {
    let (_temp, store, run_id) = run_store();
    let path = store.run_dir(run_id).join(INVENTORY_PATH);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{not json").unwrap();
    let error = compare(&store, run_id, "d", &texts(&["REQ-A-001"]), false).unwrap_err();
    assert!(
        format!("{error:#}").contains("parsing requirement inventory"),
        "{error:#}"
    );
}
