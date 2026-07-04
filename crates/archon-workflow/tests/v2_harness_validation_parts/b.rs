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
fn dynamic_placeholders_include_call_site_fingerprint() {
    let source = r#"
export default async function workflow(w) {
  let reviewIteration = 1;
  let review = await w.qualityGate("adversarial-review-" + reviewIteration, {
    inputs: [],
    criteria: "review current evidence"
  });
  if (review.status !== "accepted") {
    reviewIteration += 1;
    review = await w.qualityGate("adversarial-review-" + reviewIteration, {
      inputs: [review],
      criteria: "review remediated evidence"
    });
  }
  await w.finalReport("final", [review], {});
}
"#;

    let plan = validator().validate(source).unwrap();
    let reviews = plan
        .calls
        .iter()
        .filter(|call| call.id.starts_with("adversarial-review-dynamic-"))
        .collect::<Vec<_>>();

    assert_eq!(reviews.len(), 2);
    assert_ne!(reviews[0].id, reviews[1].id);
    for review in reviews {
        assert_eq!(
            review
                .options
                .extra
                .get("dynamic_id_prefix")
                .and_then(serde_json::Value::as_str),
            Some("adversarial-review-")
        );
    }
}

#[test]
fn duplicate_static_host_call_ids_are_rejected() {
    let err = validator()
        .validate(
            r#"
export default async function workflow(w) {
  await w.agent("duplicate", { role: "researcher", task: "first" });
  await w.agent("duplicate", { role: "researcher", task: "second" });
}
"#,
        )
        .unwrap_err();

    assert_eq!(
        err,
        WorkflowV2HarnessError::DuplicateHostCallId("duplicate".to_string())
    );
}

#[test]
fn source_aliases_follow_source_order_and_safe_reassignment() {
    let source = r#"
export default async function workflow(w) {
  let reviewIteration = 1;
  let review = await w.qualityGate("adversarial-review-" + reviewIteration, {
    inputs: [],
    criteria: "review current evidence"
  });
  const firstInventory = await w.reduce("first-review-remediation-inventory", review.outcomes, {
    role: "planner",
    task: "plan first remediation"
  });
  if (firstInventory.status !== "accepted") {
    reviewIteration += 1;
    review = await w.qualityGate("adversarial-review-" + reviewIteration, {
      inputs: [firstInventory],
      criteria: "review remediated evidence"
    });
    await w.reduce("second-review-remediation-inventory", review.outcomes, {
      role: "planner",
      task: "plan second remediation"
    });
  }
}
"#;

    let plan = validator().validate(source).unwrap();
    let reviews = plan
        .calls
        .iter()
        .filter(|call| call.id.starts_with("adversarial-review-dynamic-"))
        .collect::<Vec<_>>();
    assert_eq!(reviews.len(), 2);

    let first_inventory = plan
        .calls
        .iter()
        .find(|call| call.id == "first-review-remediation-inventory")
        .expect("first remediation inventory");
    let second_inventory = plan
        .calls
        .iter()
        .find(|call| call.id == "second-review-remediation-inventory")
        .expect("second remediation inventory");
    let first_source = format!("{}.outcomes", reviews[0].id);
    let second_source = format!("{}.outcomes", reviews[1].id);

    assert_eq!(
        first_inventory.options.source.as_deref(),
        Some(first_source.as_str())
    );
    assert_eq!(
        second_inventory.options.source.as_deref(),
        Some(second_source.as_str())
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
