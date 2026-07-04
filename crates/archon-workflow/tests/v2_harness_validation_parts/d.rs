#[test]
fn fanout_can_use_script_owned_inventory_variables() {
    let source = r#"
export default async function workflow(w) {
  const discoveryTracks = [
    { id: "prd", task: "read task files" },
    { id: "repo", task: "inspect repository" }
  ];
  const discovery = await w.parallel("initial-readonly-discovery", discoveryTracks, {
    tier: "researcher",
    task: "run read-only discovery"
  });
  return discovery;
}
"#;

    let plan = validator().validate(source).unwrap();
    let discovery = plan
        .calls
        .iter()
        .find(|call| call.id == "initial-readonly-discovery")
        .unwrap();

    assert_eq!(discovery.options.source.as_deref(), Some("discoveryTracks"));
    assert_eq!(discovery.write_mode, None);
}

#[test]
fn fanout_can_use_typed_static_items_wrapper_over_script_variable() {
    let source = r#"
export default async function workflow(w) {
  const discoveryItems = [
    { id: "prd", task: "read task files" },
    { id: "repo", task: "inspect repository" }
  ];
  const discovery = await w.fanout("read_only_discovery", {
    type: "static_items",
    items: discoveryItems
  }, {
    tier: "researcher",
    task: "run read-only discovery"
  });
  return discovery;
}
"#;

    let plan = validator().validate(source).unwrap();
    let discovery = plan
        .calls
        .iter()
        .find(|call| call.id == "read_only_discovery")
        .unwrap();

    assert!(
        discovery
            .options
            .source
            .as_deref()
            .is_some_and(|source| source.contains("items: discoveryItems"))
    );
    assert_eq!(discovery.write_mode, None);
}

#[test]
fn parallel_can_use_typed_static_items_wrapper_over_script_variable() {
    let source = r#"
export default async function workflow(w) {
  const discoveryItems = [
    { id: "prd", task: "read task files" },
    { id: "repo", task: "inspect repository" }
  ];
  await w.parallel("read_only_discovery", {
    type: "static_items",
    items: discoveryItems
  }, {
    tier: "researcher",
    task: "run read-only discovery"
  });
}
"#;

    let plan = validator().validate(source).unwrap();
    let discovery = plan
        .calls
        .iter()
        .find(|call| call.id == "read_only_discovery")
        .unwrap();

    assert_eq!(discovery.method, WorkflowV2HostMethod::Parallel);
    assert!(
        discovery
            .options
            .source
            .as_deref()
            .is_some_and(|source| source.contains("items: discoveryItems"))
    );
}

#[test]
fn reduce_inputs_can_mix_script_context_and_host_results() {
    let source = r#"
export default async function workflow(w) {
  const rootContext = {
    repository: "/repo",
    requiredReads: ["/repo/README.md"]
  };
  const discoveryPlan = await w.agent("prepare-readonly-discovery-plan", {
    tier: "planner",
    inputs: [rootContext],
    task: "plan discovery"
  });
  const discovery = await w.parallel("initial-readonly-discovery", discoveryPlan, {
    tier: "researcher",
    task: "run discovery"
  });
  const inventory = await w.reduce("dependency-aware-implementation-inventory", {
    tier: "reducer",
    inputs: [rootContext, discovery],
    task: "merge context and discovery"
  });
  return inventory;
}
"#;

    let plan = validator().validate(source).unwrap();
    let inventory = plan
        .calls
        .iter()
        .find(|call| call.id == "dependency-aware-implementation-inventory")
        .unwrap();

    assert_eq!(
        inventory.options.source.as_deref(),
        Some("[rootContext, initial-readonly-discovery]")
    );
}

#[test]
fn write_fanout_with_explicit_write_mode_is_accepted() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "researcher" });
  await w.fanout("implement", inventory.items, {
    role: "coder",
    write: "coordinated",
    task: "edit files"
  });
}
"#;

    let plan = validator().validate(source).unwrap();
    let implement = plan
        .calls
        .iter()
        .find(|call| call.id == "implement")
        .unwrap();

    assert_eq!(implement.write_mode, Some(WorkflowV2WriteMode::Coordinated));
}

#[test]
fn read_only_fanout_with_explicit_none_write_mode_is_accepted() {
    let source = r#"
export default async function workflow(w) {
  const seeds = await w.agent("discovery-seeds", { tier: "planner", task: "plan read-only discovery" });
  await w.fanout("read-only-discovery", seeds.items, {
    tier: "researcher",
    itemKind: "discovery_track",
    write: "none",
    task: "inspect only"
  });
}
"#;

    let plan = validator().validate(source).unwrap();
    let discovery = plan
        .calls
        .iter()
        .find(|call| call.id == "read-only-discovery")
        .unwrap();

    assert_eq!(discovery.write_mode, None);
    assert_eq!(
        discovery.options.source.as_deref(),
        Some("discovery-seeds.items")
    );
}

#[test]
fn write_fanout_with_invalid_write_mode_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "researcher" });
  await w.fanout("implement", inventory.items, {
    role: "coder",
    write: "unsafe",
    task: "edit files"
  });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::InvalidWriteMode {
            method: "fanout".to_string(),
            id: "implement".to_string(),
            value: "unsafe".to_string(),
        }
    );
}

#[test]
fn fanout_over_null_source_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.fanout("review", null, { role: "critic", task: "review" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::UntypedFanout("review".to_string())
    );
}

#[test]
fn fanout_with_object_runtime_source_is_accepted_by_safety_validation() {
    let plan = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.fanout("review", { role: "critic", task: "review" });
}"#,
        )
        .unwrap();

    assert_eq!(plan.calls[0].id, "review");
    assert_eq!(
        plan.calls[0].options.source.as_deref(),
        Some("{ role: \"critic\", task: \"review\" }")
    );
}

#[test]
fn parallel_with_object_runtime_source_is_accepted_by_safety_validation() {
    let plan = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.parallel("review", { role: "critic", task: "review" });
}"#,
        )
        .unwrap();

    assert_eq!(plan.calls[0].id, "review");
    assert_eq!(
        plan.calls[0].options.source.as_deref(),
        Some("{ role: \"critic\", task: \"review\" }")
    );
}

#[test]
fn fanout_over_literal_string_source_is_runtime_data_not_schema_validation() {
    let plan = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.fanout("review", "not typed", { role: "critic", task: "review" });
}"#,
        )
        .unwrap();

    assert_eq!(
        plan.calls[0].options.source.as_deref(),
        Some("\"not typed\"")
    );
}

#[test]
fn fanout_over_inline_array_source_is_accepted() {
    let plan = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.fanout("review", [{ id: "one" }], { role: "critic", task: "review" });
}"#,
        )
        .unwrap();

    assert_eq!(
        plan.calls[0].options.source.as_deref(),
        Some("[{ id: \"one\" }]")
    );
}
