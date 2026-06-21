use archon_workflow::{HarnessCompiler, StageKind};

#[test]
fn harness_compiles_read_only_audit_without_implementation() {
    let source = r#"
export default async function workflow(w) {
  const items = await w.agent("discover", { tier: "planner", task: "inventory" });
  const reviews = await w.fanout("review", items, { tier: "critic", maxParallelism: 4 });
  const report = await w.reduce("synthesize", reviews, { kind: "evidence_weighted_report" });
  await w.qualityGate("quality", report);
}
"#;

    let spec = HarnessCompiler::default()
        .compile(source, "Repo audit", "Audit repository behavior")
        .unwrap();

    assert_eq!(spec.stages.len(), 4);
    assert!(
        spec.stages
            .iter()
            .all(|stage| stage.kind != StageKind::Implementation),
        "read-only audit harness must not compile to implementation stages"
    );
    let discover = spec
        .stages
        .iter()
        .find(|stage| stage.id == "discover")
        .unwrap();
    assert!(
        discover
            .extra
            .get("outputs")
            .and_then(|value| value.as_array())
            .is_some_and(|outputs| outputs.iter().any(|value| value.as_str() == Some("items"))),
        "fanout source must declare machine-readable items output"
    );
    assert!(
        discover
            .task
            .as_deref()
            .is_some_and(|task| task.contains("Structured item output contract")),
        "fanout source task must require parseable items output"
    );
    assert_eq!(spec.stages[1].foreach.as_deref(), Some("${discover.items}"));
}

#[test]
fn harness_rejects_unsafe_javascript_surfaces() {
    for source in [
        r#"import fs from "fs"; export default async function workflow(w) { await w.agent("a", {}); }"#,
        r#"export default async function workflow(w) { eval("w.agent('a', {})"); }"#,
        r#"export default async function workflow(w) { await fetch("https://example.com"); }"#,
        r#"export default async function workflow(w) { await w.agent("a", { provider: "anthropic" }); }"#,
        r#"export default async function workflow(w) { await w.agent("a", { model : "gpt-anything" }); }"#,
        r#"export default async function workflow(w) { await w.shell("a", {}); }"#,
    ] {
        assert!(
            HarnessCompiler::default().validate(source).is_err(),
            "unsafe harness should be rejected: {source}"
        );
    }
}

#[test]
fn harness_ignores_comments_when_scanning_for_unsafe_tokens_or_calls() {
    let source = r#"
export default async function workflow(w) {
  // Forbidden words in comments are documentation, not executable code:
  // import fs, require("x"), eval("x"), new Function("x"), fetch("https://example.com")
  // Fake examples should not become phases: await w.agent("comment-only", {});
  const result = await w.agent("discover", { tier: "planner", task: "inventory" });
  await w.qualityGate("quality", result);
}
"#;

    let phases = HarnessCompiler::default().validate(source).unwrap();
    assert_eq!(phases.len(), 2);
    assert!(phases.iter().any(|phase| phase.id == "discover"));
    assert!(!phases.iter().any(|phase| phase.id == "comment-only"));
}

#[test]
fn harness_compiles_single_edit_with_explicit_targets_only() {
    let source = r#"
export default async function workflow(w) {
  await w.implementation("edit-lib", {
    tier: "coder",
    task: "edit src/lib.rs",
    targetFiles: ["src/lib.rs"],
    verifyCommand: "cargo test -p archon-workflow harness_compiles_single_edit"
  });
  await w.qualityGate("quality", { depends_on: ["edit-lib"] });
}
"#;

    let spec = HarnessCompiler::default()
        .compile(source, "Small edit", "Implement a focused change")
        .unwrap();

    let edit = spec
        .stages
        .iter()
        .find(|stage| stage.id == "edit-lib")
        .unwrap();
    assert_eq!(edit.kind, StageKind::Implementation);
    assert_eq!(edit.expected_target_files, vec!["src/lib.rs"]);
    assert_eq!(
        edit.verify_command.as_deref(),
        Some("cargo test -p archon-workflow harness_compiles_single_edit")
    );
}

#[test]
fn harness_rejects_implementation_for_non_edit_tasks() {
    let source = r#"
export default async function workflow(w) {
  await w.implementation("edit-lib", {
    tier: "coder",
    task: "edit src/lib.rs",
    targetFiles: ["src/lib.rs"]
  });
}
"#;

    let err = HarnessCompiler::default()
        .compile(source, "Audit", "Audit repository behavior")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("non-editing task"),
        "unexpected compiler error: {err}"
    );
}

#[test]
fn harness_rejects_direct_implementation_without_targets() {
    let source = r#"
export default async function workflow(w) {
  await w.implementation("edit-lib", { tier: "coder", task: "edit src/lib.rs" });
}
"#;

    let err = HarnessCompiler::default()
        .compile(source, "Small edit", "Implement a focused change")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("requires targetFiles"),
        "unexpected compiler error: {err}"
    );
}

#[test]
fn harness_rejects_implementation_fanout_without_item_targets() {
    let source = r#"
export default async function workflow(w) {
  const items = await w.agent("discover", { tier: "planner", task: "inventory" });
  await w.fanout("migrate", items, { tier: "coder", itemKind: "implementation" });
}
"#;

    let err = HarnessCompiler::default()
        .compile(source, "Migration", "Migrate repository code")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("targetFiles"),
        "unexpected compiler error: {err}"
    );
}

#[test]
fn harness_compiles_migration_fanout_with_item_targets() {
    let source = r#"
export default async function workflow(w) {
  const items = await w.agent("discover", { tier: "planner", task: "emit items with target_files" });
  const edits = await w.fanout("migrate", items, {
    tier: "coder",
    maxParallelism: 8,
    itemKind: "implementation",
    targetFilesFromItem: true
  });
  const report = await w.reduce("synthesize", edits, { kind: "evidence_weighted_report" });
  await w.qualityGate("quality", report);
}
"#;

    let spec = HarnessCompiler::default()
        .compile(source, "Migration", "Migrate repository code")
        .unwrap();

    let migrate = spec
        .stages
        .iter()
        .find(|stage| stage.id == "migrate")
        .unwrap();
    let discover = spec
        .stages
        .iter()
        .find(|stage| stage.id == "discover")
        .unwrap();
    assert_eq!(migrate.kind, StageKind::Fanout);
    assert_eq!(migrate.item_kind, Some(StageKind::Implementation));
    assert_eq!(migrate.foreach.as_deref(), Some("${discover.items}"));
    assert!(
        discover
            .task
            .as_deref()
            .is_some_and(|task| task.contains("top-level `items` array")),
        "implementation fanout source task must force structured items"
    );
}

#[test]
fn harness_lowers_inline_items_and_artifact_backed_fanout_sources() {
    let source = r#"
export default async function workflow(w) {
  const discovery = await w.agent("discover", { tier: "planner", outputArtifact: "discovery.items" });
  const plan = await w.reduce("implementation-plan", {
    tier: "reducer",
    inputs: { discovery },
    outputArtifact: "tdl.plan"
  });
  await w.qualityGate("plan-quality-gate", { inputs: { plan } });
  await w.checkpoint("plan-approved", { artifacts: ["tdl.plan"] });
  const phase1 = await w.fanout("implement-T001", {
    tier: "coder",
    itemKind: "implementation",
    itemsFromArtifact: plan,
    itemFilter: { tasks: ["T001"] },
    targetFilesFromItem: true
  });
  await w.fanout("adversarial-review-T001", {
    tier: "critic",
    items: [
      { name: "prd-compliance", task: "Review PRD compliance" },
      { name: "tests", task: "Review focused tests" }
    ],
    inputs: { phase1 }
  });
}
"#;

    let spec = HarnessCompiler::default()
        .compile(source, "Migration", "Implement repository changes")
        .unwrap();

    let implement = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implement-T001")
        .unwrap();
    assert_eq!(
        implement.foreach.as_deref(),
        Some("${implementation-plan.items}")
    );
    let plan = spec
        .stages
        .iter()
        .find(|stage| stage.id == "implementation-plan")
        .unwrap();
    assert!(
        plan.task
            .as_deref()
            .is_some_and(|task| task.contains("Structured item output contract")),
        "itemsFromArtifact source must receive the item output task contract"
    );
    assert!(
        implement
            .depends_on
            .contains(&"implementation-plan".to_string())
    );
    assert!(implement.depends_on.contains(&"plan-approved".to_string()));
    assert_eq!(
        implement.filter.as_deref(),
        Some("item.task_id in ['T001']")
    );

    let review = spec
        .stages
        .iter()
        .find(|stage| stage.id == "adversarial-review-T001")
        .unwrap();
    assert!(review.foreach.is_none());
    assert_eq!(review.input["items"].as_array().unwrap().len(), 2);
}
