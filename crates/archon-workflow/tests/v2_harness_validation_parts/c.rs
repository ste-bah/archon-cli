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
  await w.tool("inspect", { tool: "checkpoint", task: "read metadata" });
  await w.implementation("implement", {
    write: "serial",
    task: "edit files",
    targetFiles: ["src/lib.rs"]
  });
}
"#;

    let plan = validator().validate(source).unwrap();

    assert_eq!(plan.calls[0].method, WorkflowV2HostMethod::Tool);
    assert_eq!(
        plan.calls[0]
            .options
            .extra
            .get("tool")
            .and_then(serde_json::Value::as_str),
        Some("checkpoint")
    );
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
fn source_expressions_are_script_owned_metadata() {
    let source = r#"
export default async function workflow(w) {
  await w.fanout("review", missing.items, { role: "critic", task: "review" });
}
"#;

    let plan = validator().validate(source).unwrap();
    let review = plan.calls.iter().find(|call| call.id == "review").unwrap();

    assert_eq!(review.options.source.as_deref(), Some("missing.items"));
}
