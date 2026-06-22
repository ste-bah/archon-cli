use archon_workflow::v2::{
    WorkflowV2HarnessError, WorkflowV2HarnessValidator, WorkflowV2HostMethod, WorkflowV2WriteMode,
};

fn validator() -> WorkflowV2HarnessValidator {
    WorkflowV2HarnessValidator
}

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
    assert_eq!(wave.options.source.as_deref(), Some("currentItems"));
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

#[test]
fn adaptive_remediation_while_loop_is_accepted() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", {
    role: "planner",
    task: "return items needing implementation"
  });
  let iteration = 1;
  let remaining = inventory.items || [];
  while (remaining.length > 0 && iteration <= 5) {
    const implementation = await w.fanout("implementation-wave-" + iteration, remaining, {
      role: "coder",
      write: "coordinated",
      itemKind: "implementation",
      targetFilesFromItem: true,
      maxParallelism: 4,
      task: "implement one item"
    });
    const review = await w.reduce("adversarial-review-" + iteration, implementation, {
      role: "critic",
      task: "return remaining issues as top-level items"
    });
    remaining = review.items || [];
    iteration += 1;
  }
  await w.finalReport("final", [inventory], {});
}
"#;

    let plan = validator().validate(source).unwrap();
    let implementation = plan
        .calls
        .iter()
        .find(|call| call.id.starts_with("implementation-wave-dynamic-"))
        .unwrap();
    let review = plan
        .calls
        .iter()
        .find(|call| call.id.starts_with("adversarial-review-dynamic-"))
        .unwrap();

    assert_eq!(implementation.method, WorkflowV2HostMethod::Fanout);
    assert_eq!(
        implementation.write_mode,
        Some(WorkflowV2WriteMode::Coordinated)
    );
    assert_eq!(implementation.options.source.as_deref(), Some("remaining"));
    assert_eq!(
        implementation
            .options
            .extra
            .get("runtime_loop")
            .and_then(serde_json::Value::as_str),
        Some("while")
    );
    assert_eq!(review.method, WorkflowV2HostMethod::Reduce);
    assert_eq!(
        review.options.source.as_deref(),
        Some(implementation.id.as_str())
    );
    assert_eq!(
        review
            .options
            .extra
            .get("dynamic_template")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn adaptive_while_loop_with_static_host_call_id_is_rejected() {
    let err = validator()
        .validate(
            r#"
export default async function workflow(w) {
  let iteration = 1;
  while (iteration <= 2) {
    await w.checkpoint("same-call-every-iteration", {});
    iteration += 1;
  }
}
"#,
        )
        .unwrap_err();

    assert!(matches!(err, WorkflowV2HarnessError::UnsupportedLoop(_)));
}

#[test]
fn dynamic_template_host_call_id_is_accepted_with_stable_template_id() {
    let source = r#"
export default async function workflow(w) {
  const items = [{ id: "one" }];
  let waveIndex = 1;
  await w.fanout(`implementation-wave-${waveIndex}`, items, {
    tier: "coder",
    write: "worktree",
    itemKind: "implementation",
    targetFilesFromItem: true,
    task: "implement one item"
  });
}
"#;

    let plan = validator().validate(source).unwrap();
    let wave = plan
        .calls
        .iter()
        .find(|call| call.id.starts_with("implementation-wave-dynamic-"))
        .unwrap();

    assert_eq!(wave.options.source.as_deref(), Some("items"));
    assert_eq!(wave.write_mode, Some(WorkflowV2WriteMode::Worktree));
    assert_eq!(
        wave.options
            .extra
            .get("dynamic_id")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
fn unsafe_import_is_rejected() {
    let err = validator()
        .validate(r#"import fs from "fs"; export default async function workflow(w) {}"#)
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::ForbiddenToken("import statement")
    );
}

#[test]
fn compact_static_import_is_rejected() {
    let err = validator()
        .validate(r#"import{readFile}from"fs"; export default async function workflow(w) {}"#)
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::ForbiddenToken("import statement")
    );
}

#[test]
fn newline_static_import_is_rejected() {
    let err = validator()
        .validate(
            r#"import
{ readFile } from "fs"; export default async function workflow(w) {}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::ForbiddenToken("import statement")
    );
}

#[test]
fn dynamic_import_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await import("fs");
  await w.agent("safe", { role: "researcher" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::ForbiddenToken("dynamic import")
    );
}

#[test]
fn direct_shell_or_process_escape_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  child_process.exec("rm -rf .");
  await w.agent("safe", { role: "researcher" });
}"#,
        )
        .unwrap_err();

    assert_eq!(err, WorkflowV2HarnessError::ForbiddenToken("child_process"));
}

#[test]
fn unsafe_tokens_inside_task_strings_are_not_treated_as_code() {
    let source = r#"
export default async function workflow(w) {
  await w.agent("inspect", {
    role: "researcher",
    task: "Keep new functions low complexity; do not import modules or call fetch."
  });
}
"#;

    let plan = validator().validate(source).unwrap();

    assert_eq!(plan.calls.len(), 1);
    assert_eq!(plan.calls[0].id, "inspect");
}

#[test]
fn direct_network_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await fetch("https://example.com");
  await w.agent("safe", { role: "researcher" });
}"#,
        )
        .unwrap_err();

    assert_eq!(err, WorkflowV2HarnessError::ForbiddenToken("fetch("));
}

#[test]
fn host_calls_without_literal_string_ids_are_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  const discovery = await w.agent("discover", { role: "researcher" });
  await w.fanout(discovery.items, { role: "critic", task: "review" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::HostCallRequiresLiteralId("fanout".to_string())
    );
}

#[test]
fn host_api_method_aliases_are_rejected() {
    let err = validator()
        .validate(
            r#"
export default async function workflow(w) {
  await w.agent("visible", { role: "researcher", task: "visible phase" });
  const call = w.agent;
  await call("hidden", { role: "researcher", task: "hidden phase" });
}
"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::ForbiddenToken("host API reference outside direct call")
    );
}

#[test]
fn host_api_object_aliases_are_rejected() {
    let err = validator()
        .validate(
            r#"
export default async function workflow(w) {
  await w.agent("visible", { role: "researcher", task: "visible phase" });
  const host = w;
  await host.agent("hidden", { role: "researcher", task: "hidden phase" });
}
"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::ForbiddenToken("host API alias")
    );
}

#[test]
fn host_api_bracket_access_is_rejected() {
    let err = validator()
        .validate(
            r#"
export default async function workflow(w) {
  await w["agent"]("hidden", { role: "researcher", task: "hidden phase" });
}
"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::ForbiddenToken("host API bracket access")
    );
}

#[test]
fn unsupported_host_call_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.shell("bad", {});
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::UnsupportedHostMethod("shell".to_string())
    );
}

#[test]
fn provider_or_model_literals_are_rejected() {
    let provider_err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.agent("bad", { provider : "auto" });
}"#,
        )
        .unwrap_err();
    let model_err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.agent("bad", { model : "sonnet" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        provider_err,
        WorkflowV2HarnessError::ForbiddenToken("provider literal")
    );
    assert_eq!(
        model_err,
        WorkflowV2HarnessError::ForbiddenToken("model literal")
    );
}

#[test]
fn write_fanout_without_write_mode_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "researcher" });
  await w.fanout("implement", inventory.items, { role: "coder", task: "edit files" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::MissingWriteMode {
            method: "fanout".to_string(),
            id: "implement".to_string(),
        }
    );
}

#[test]
fn write_agent_without_write_mode_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.agent("implement", { role: "coder", task: "edit files" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::MissingWriteMode {
            method: "agent".to_string(),
            id: "implement".to_string(),
        }
    );
}

#[test]
fn direct_implementation_without_write_mode_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.implementation("implement", { task: "edit files", targetFiles: ["src/lib.rs"] });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::MissingWriteMode {
            method: "implementation".to_string(),
            id: "implement".to_string(),
        }
    );
}

#[test]
fn direct_implementation_and_tool_calls_are_supported() {
    let source = r#"
export default async function workflow(w) {
  await w.tool("inspect", { task: "read metadata" });
  await w.implementation("implement", {
    write: "serial",
    task: "edit files",
    targetFiles: ["src/lib.rs"]
  });
}
"#;

    let plan = validator().validate(source).unwrap();

    assert_eq!(plan.calls[0].method, WorkflowV2HostMethod::Tool);
    assert_eq!(plan.calls[1].method, WorkflowV2HostMethod::Implementation);
    assert_eq!(plan.calls[1].write_mode, Some(WorkflowV2WriteMode::Serial));
}

#[test]
fn required_host_api_calls_are_supported() {
    let source = r#"
export default async function workflow(w) {
  const checkpoint = await w.checkpoint("checkpoint", { status: "started" });
  const artifact = await w.saveArtifact("artifact", checkpoint, {});
  await w.requireArtifact("require-artifact", artifact, {});
  const quality = await w.qualityGate("quality", [checkpoint, artifact], {});
  await w.humanGate("human", { task: "manual approval" });
  await w.finalReport("final", [quality], {});
}
"#;

    let plan = validator().validate(source).unwrap();

    assert_eq!(
        plan.calls
            .iter()
            .map(|call| call.method)
            .collect::<Vec<_>>(),
        vec![
            WorkflowV2HostMethod::Checkpoint,
            WorkflowV2HostMethod::SaveArtifact,
            WorkflowV2HostMethod::RequireArtifact,
            WorkflowV2HostMethod::QualityGate,
            WorkflowV2HostMethod::HumanGate,
            WorkflowV2HostMethod::FinalReport,
        ]
    );
    assert_eq!(
        plan.calls[0].options.source.as_deref(),
        Some(r#"{ status: "started" }"#)
    );
    assert_eq!(plan.calls[1].options.source.as_deref(), Some("checkpoint"));
    assert_eq!(plan.calls[2].options.source.as_deref(), Some("artifact"));
    assert_eq!(
        plan.calls[3].options.source.as_deref(),
        Some("[checkpoint, artifact]")
    );
    assert_eq!(plan.calls[5].options.source.as_deref(), Some("[quality]"));
}

#[test]
fn host_call_options_capture_runtime_fanout_metadata() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "researcher", task: "return data.items" });
  await w.fanout("review", inventory.items, {
    role: "critic",
    task: "review one typed item",
    targetFilesFromItem: true,
    targetFiles: ["src/lib.rs"],
    maxParallelism: 4
  });
}
"#;

    let plan = validator().validate(source).unwrap();
    let review = plan.calls.iter().find(|call| call.id == "review").unwrap();

    assert_eq!(review.options.source.as_deref(), Some("inventory.items"));
    assert_eq!(review.options.role.as_deref(), Some("critic"));
    assert_eq!(
        review.options.task.as_deref(),
        Some("review one typed item")
    );
    assert_eq!(review.options.target_files, vec!["src/lib.rs"]);
    assert!(review.options.target_files_from_item);
    assert_eq!(review.options.max_parallelism, Some(4));
}

#[test]
fn host_call_options_capture_runtime_parallel_metadata() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "researcher", task: "return data.items" });
  await w.parallel("review", inventory.items, {
    role: "critic",
    task: "review one typed item",
    maxParallelism: 4
  });
}
"#;

    let plan = validator().validate(source).unwrap();
    let review = plan.calls.iter().find(|call| call.id == "review").unwrap();

    assert_eq!(review.method, WorkflowV2HostMethod::Parallel);
    assert_eq!(review.options.source.as_deref(), Some("inventory.items"));
    assert_eq!(review.options.role.as_deref(), Some("critic"));
    assert_eq!(review.options.max_parallelism, Some(4));
}

#[test]
fn host_call_sources_rewrite_javascript_bindings_to_call_ids() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "researcher", task: "return data.items" });
  const reviews = await w.fanout("review", inventory.items, { role: "critic", task: "review" });
  await w.reduce("final", [inventory, reviews], { role: "reducer", task: "synthesize" });
}
"#;

    let plan = validator().validate(source).unwrap();
    let review = plan.calls.iter().find(|call| call.id == "review").unwrap();
    let final_report = plan.calls.iter().find(|call| call.id == "final").unwrap();

    assert_eq!(review.options.binding.as_deref(), Some("reviews"));
    assert_eq!(review.options.source.as_deref(), Some("inventory.items"));
    assert_eq!(
        final_report.options.source.as_deref(),
        Some("[inventory, review]")
    );
}

#[test]
fn object_option_inputs_are_captured_as_sources() {
    let source = r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "researcher", task: "return data.items" });
  const implementation = await w.fanout("implement", inventory.items, {
    role: "coder",
    itemKind: "implementation",
    write: "coordinated",
    targetFilesFromItem: true,
    task: "edit files"
  });
  const review = await w.reduce("review", {
    role: "critic",
    inputs: [inventory, implementation],
    task: "review implementation evidence"
  });
  await w.qualityGate("gate", {
    inputs: [review],
    criteria: "review passed"
  });
  const artifact = await w.saveArtifact("artifact", {
    artifact: review,
    path: "artifacts/review.json"
  });
  await w.requireArtifact("require-artifact", {
    artifact: artifact,
    path: "artifacts/review.json"
  });
}
"#;

    let plan = validator().validate(source).unwrap();
    let review = plan.calls.iter().find(|call| call.id == "review").unwrap();
    let gate = plan.calls.iter().find(|call| call.id == "gate").unwrap();
    let artifact = plan
        .calls
        .iter()
        .find(|call| call.id == "artifact")
        .unwrap();
    let require = plan
        .calls
        .iter()
        .find(|call| call.id == "require-artifact")
        .unwrap();

    assert_eq!(
        review.options.source.as_deref(),
        Some("[inventory, implement]")
    );
    assert_eq!(gate.options.source.as_deref(), Some("[review]"));
    assert_eq!(artifact.options.source.as_deref(), Some("review"));
    assert_eq!(require.options.source.as_deref(), Some("artifact"));
}

#[test]
fn checkpoint_object_inputs_are_captured_without_losing_plain_metadata_checkpoints() {
    let source = r#"
export default async function workflow(w) {
  const discovery = await w.agent("discover", { role: "researcher", task: "inspect" });
  const gate = await w.qualityGate("gate", {
    inputs: [discovery],
    criteria: "accepted"
  });
  await w.checkpoint("complete", {
    inputs: [gate],
    state: "completed"
  });
  await w.checkpoint("metadata-only", {
    state: "started"
  });
}
"#;

    let plan = validator().validate(source).unwrap();
    let complete = plan
        .calls
        .iter()
        .find(|call| call.id == "complete")
        .unwrap();
    let metadata = plan
        .calls
        .iter()
        .find(|call| call.id == "metadata-only")
        .unwrap();

    assert_eq!(complete.options.source.as_deref(), Some("[gate]"));
    let metadata_source = metadata.options.source.as_deref().unwrap();
    assert!(metadata_source.starts_with('{'));
    assert!(metadata_source.contains(r#"state: "started""#));
}

#[test]
fn host_call_sources_must_reference_earlier_calls() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.fanout("review", missing.items, { role: "critic", task: "review" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::UnknownSource {
            method: "fanout".to_string(),
            id: "review".to_string(),
            source_id: "missing".to_string(),
        }
    );
}

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

    assert_eq!(discovery.options.source.as_deref(), Some("discoveryItems"));
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
    assert_eq!(discovery.options.source.as_deref(), Some("discoveryItems"));
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
fn fanout_without_typed_item_source_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.fanout("review", { role: "critic", task: "review" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::UntypedFanout("review".to_string())
    );
}

#[test]
fn parallel_without_typed_item_source_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.parallel("review", { role: "critic", task: "review" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::UntypedFanout("review".to_string())
    );
}

#[test]
fn fanout_over_literal_string_source_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.fanout("review", "not typed", { role: "critic", task: "review" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::UntypedFanout("review".to_string())
    );
}

#[test]
fn fanout_over_inline_array_source_is_rejected() {
    let err = validator()
        .validate(
            r#"export default async function workflow(w) {
  await w.fanout("review", [{ id: "one" }], { role: "critic", task: "review" });
}"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::UntypedFanout("review".to_string())
    );
}
