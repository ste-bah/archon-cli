#[test]
fn valid_claude_style_harness_is_accepted() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "researcher", task: "read task files" });
  const reviews = await w.fanout("review", inventory.items, { role: "critic", task: "review" });
  const report = await w.reduce("report", reviews, { reducer: "evidence_weighted_report" });
  return await w.finalReport("final", [report], {});
}
"#;

    let plan = validator().validate(source).unwrap();

    assert_eq!(plan.calls.len(), 4);
    assert_eq!(plan.calls[0].method, WorkflowV2HostMethod::Agent);
    assert_eq!(plan.calls[1].method, WorkflowV2HostMethod::Fanout);
    assert_eq!(plan.calls[3].method, WorkflowV2HostMethod::FinalReport);
}

#[test]
fn fanout_accepts_actual_javascript_array_source() {
    let source = r#"
export default async function workflow(w) {
  const discoveryItems = [
    { id: "prd", path: "PRD.md" },
    { id: "tasks", path: "tasks/README.md" }
  ];
  const discovery = await w.parallel("read-only-discovery", discoveryItems, {
    role: "researcher",
    task: "inspect one source item"
  });
  await w.reduce("inventory", discovery.items, {
    role: "reducer",
    task: "merge branch findings"
  });
}
"#;

    let plan = validator().validate(source).unwrap();
    let discovery = plan
        .calls
        .iter()
        .find(|call| call.id == "read-only-discovery")
        .expect("discovery fanout");

    assert_eq!(discovery.method, WorkflowV2HostMethod::Parallel);
    assert_eq!(discovery.options.source.as_deref(), Some("discoveryItems"));
}

#[test]
fn nested_helper_host_calls_do_not_become_executable_phases() {
    let source = r#"
export default async function workflow(w) {
  async function helper() {
    await w.agent("helper-hidden", { role: "researcher", task: "not executable" });
  }
  const inventory = await w.agent("inventory", { role: "researcher", task: "read task files" });
  return inventory;
}
"#;

    let plan = validator().validate(source).unwrap();

    assert_eq!(plan.calls.len(), 1);
    assert_eq!(plan.calls[0].id, "inventory");
}

#[test]
fn hidden_helper_only_host_calls_are_rejected() {
    let err = validator()
        .validate(
            r#"
export default async function workflow(w) {
  async function helper() {
    return await w.agent("helper-hidden", { role: "researcher", task: "not visible" });
  }
  return { planned: true };
}
"#,
        )
        .unwrap_err();

    assert_eq!(err, WorkflowV2HarnessError::NoHostCalls);
}

#[test]
fn literal_if_branches_follow_executable_control_flow() {
    let source = r#"
export default async function workflow(w) {
  if (false) {
    await w.agent("dead-branch", { role: "researcher", task: "dead" });
  } else {
    const fallback = await w.agent("fallback", { role: "researcher", task: "live" });
    await w.finalReport("final", [fallback], {});
  }
}
"#;

    let plan = validator().validate(source).unwrap();

    assert_eq!(
        plan.calls
            .iter()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>(),
        vec!["fallback", "final"]
    );
}

#[test]
fn dynamic_if_branches_are_annotated_with_rewritten_conditions() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "return items" });
  if (inventory.items.length > 0) {
    const implemented = await w.fanout("implementation", inventory.items, {
      role: "coder",
      itemKind: "implementation",
      write: "coordinated",
      targetFilesFromItem: true,
      task: "implement one item"
    });
    await w.finalReport("final", [implemented], {});
  } else {
    await w.finalReport("noop", [inventory], {});
  }
}
"#;

    let plan = validator().validate(source).unwrap();

    let implementation = plan
        .calls
        .iter()
        .find(|call| call.id == "implementation")
        .expect("implementation branch");
    assert_eq!(
        implementation
            .options
            .extra
            .get("condition")
            .and_then(serde_json::Value::as_str),
        Some("inventory.items.length > 0")
    );
    let noop = plan
        .calls
        .iter()
        .find(|call| call.id == "noop")
        .expect("noop branch");
    assert_eq!(
        noop.options
            .extra
            .get("condition")
            .and_then(serde_json::Value::as_str),
        Some("!(inventory.items.length > 0)")
    );
}

#[test]
fn condition_variables_bound_to_hyphenated_call_ids_stay_script_owned() {
    let source = r#"
export default async function workflow(w) {
  const discoveryItems = [{ id: "repo" }];
  const discovery = await w.parallel("readonly-discovery", discoveryItems, {
    tier: "researcher",
    task: "inspect"
  });
  const preflight = await w.qualityGate("preflight-readiness", {
    inputs: [discovery],
    task: "review readiness"
  });
  if (preflight.status !== "accepted") {
    await w.agent("preflight-remediation-plan", {
      tier: "planner",
      inputs: [preflight, discovery],
      task: "repair plan"
    });
  }
}
"#;

    let plan = validator().validate(source).unwrap();
    let remediation = plan
        .calls
        .iter()
        .find(|call| call.id == "preflight-remediation-plan")
        .expect("remediation call");

    assert_eq!(
        remediation
            .options
            .extra
            .get("condition")
            .and_then(serde_json::Value::as_str),
        Some("preflight.status !== \"accepted\"")
    );
    assert_eq!(
        remediation.options.source.as_deref(),
        Some("[preflight-readiness, readonly-discovery]")
    );
}

#[test]
fn condition_expressions_are_metadata_not_static_sources() {
    let source = r#"
export default async function workflow(w) {
  const discovery = await w.agent("discover", {
    tier: "researcher",
    task: "inspect"
  });
  if (Date.now() > 0 && discovery.status !== "cancelled") {
    await w.finalReport("final", [discovery], {});
  }
}
"#;

    let plan = validator().validate(source).unwrap();
    let final_report = plan
        .calls
        .iter()
        .find(|call| call.id == "final")
        .expect("final report");

    assert_eq!(
        final_report
            .options
            .extra
            .get("condition")
            .and_then(serde_json::Value::as_str),
        Some("Date.now() > 0 && discovery.status !== \"cancelled\"")
    );
}

#[test]
fn for_of_agent_loop_lowers_to_source_fanout() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "return review items" });
  for (const item of inventory.items) {
    await w.agent("review", { role: "critic", task: `Review ${item.path}` });
  }
}
"#;

    let plan = validator().validate(source).unwrap();

    assert_eq!(plan.calls.len(), 2);
    assert_eq!(plan.calls[1].id, "review");
    assert_eq!(plan.calls[1].method, WorkflowV2HostMethod::Fanout);
    assert_eq!(
        plan.calls[1].options.source.as_deref(),
        Some("inventory.items")
    );
    assert_eq!(
        plan.calls[1]
            .options
            .extra
            .get("loop_source")
            .and_then(serde_json::Value::as_str),
        Some("inventory.items")
    );
}

#[test]
fn for_of_implementation_loop_lowers_to_worktree_fanout() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "return implementation items" });
  for (const item of inventory.items) {
    await w.implementation("implementation", {
      role: "coder",
      write: "worktree",
      itemKind: "implementation",
      targetFilesFromItem: true,
      task: `Implement ${item.id}`
    });
  }
}
"#;

    let plan = validator().validate(source).unwrap();

    assert_eq!(plan.calls.len(), 2);
    assert_eq!(plan.calls[1].id, "implementation");
    assert_eq!(plan.calls[1].method, WorkflowV2HostMethod::Fanout);
    assert_eq!(
        plan.calls[1].write_mode,
        Some(WorkflowV2WriteMode::Worktree)
    );
    assert_eq!(
        plan.calls[1].options.source.as_deref(),
        Some("inventory.items")
    );
    assert!(plan.calls[1].options.target_files_from_item);
}

#[test]
fn indexed_loop_with_dynamic_host_call_ids_is_accepted() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "return items" });
  for (let i = 0; i < inventory.items.length; i++) {
    await w.agent("review-" + i, { role: "critic", task: "review" });
  }
}
"#;

    let plan = validator().validate(source).unwrap();
    let review = plan
        .calls
        .iter()
        .find(|call| call.id.starts_with("review-dynamic-"))
        .unwrap();

    assert_eq!(review.method, WorkflowV2HostMethod::Agent);
    assert_eq!(
        review
            .options
            .extra
            .get("runtime_loop")
            .and_then(serde_json::Value::as_str),
        Some("for")
    );
    assert_eq!(
        review
            .options
            .extra
            .get("loop_header")
            .and_then(serde_json::Value::as_str),
        Some("let i = 0; i < inventory.items.length; i++")
    );
    assert_eq!(
        review
            .options
            .extra
            .get("dynamic_id")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn indexed_loop_with_static_host_call_id_is_rejected() {
    let err = validator()
        .validate(
            r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "return items" });
  for (let i = 0; i < inventory.items.length; i++) {
    await w.agent("review", { role: "critic", task: "review" });
  }
}
"#,
        )
        .unwrap_err();

    assert!(matches!(err, WorkflowV2HarnessError::UnsupportedLoop(_)));
}

#[test]
fn dynamic_loop_fanout_id_with_static_items_source_is_accepted() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.reduce("dependency-ordered-gap-inventory", {
    tier: "reducer",
    task: "return data.items"
  });
  const completedWorkUnitIds = [];
  let remainingItems = inventory.items || [];
  let waveIndex = 1;

  while (remainingItems.length > 0 && waveIndex <= 6) {
    const readyItems = remainingItems.filter(item => {
      const deps = Array.isArray(item.dependency_ids) ? item.dependency_ids : [];
      return deps.every(dep => completedWorkUnitIds.indexOf(dep) >= 0);
    });
    const currentItems = readyItems.length > 0 ? readyItems : remainingItems;
    const wave = await w.fanout("implementation-wave-" + waveIndex, {
      type: "static_items",
      items: currentItems
    }, {
      tier: "coder",
      write: "coordinated",
      itemKind: "implementation",
      targetFilesFromItem: true,
      maxParallelism: 4,
      task: "implement one dependency-ready item"
    });
    remainingItems = remainingItems.filter(item => item.id !== wave.id);
    waveIndex += 1;
  }
}
"#;

    let plan = validator().validate(source).unwrap();
    let wave = plan
        .calls
        .iter()
        .find(|call| call.id.starts_with("implementation-wave-dynamic-"))
        .unwrap();

    assert_eq!(wave.method, WorkflowV2HostMethod::Fanout);
    assert!(
        wave.options
            .source
            .as_deref()
            .is_some_and(|source| source.contains("items: currentItems"))
    );
    assert_eq!(wave.write_mode, Some(WorkflowV2WriteMode::Coordinated));
    assert!(wave.options.target_files_from_item);
    assert_eq!(
        wave.options
            .extra
            .get("runtime_loop")
            .and_then(serde_json::Value::as_str),
        Some("while")
    );
    assert_eq!(
        wave.options
            .extra
            .get("dynamic_id")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}
