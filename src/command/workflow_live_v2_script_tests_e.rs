#[tokio::test]
async fn workless_authored_script_is_rejected_before_live_execution() {
    // The canned author only ever produces a phase/log-only script — the
    // pre-flight must reject it twice and refuse to execute live.
    let workless = r#"export const meta = { name: 'workless-demo', phases: [{ title: 'Only' }] }
export default async function workflow({ phase, log }) {
  await phase("No Real Work");
  await log("nothing spawned");
  return { accepted: [], blocked: [], notes: "did nothing" };
}
"#;
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        Arc::new(CannedAuthorLlm {
            script: workless.to_string(),
        }),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "workless author".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store,
        workflow_store.clone(),
        run.id.clone(),
        true,
        None,
        None,
    );
    let authored_path = workflow_store.run_dir(&run.id).join("authored-workflow.js");

    let error = runner
        .run_authored_script_lifecycle(authored_path.clone(), serde_json::Value::Null)
        .await
        .expect_err("workless script must be rejected");

    let message = error.to_string();
    assert!(
        message.contains("dry-run pre-flight twice"),
        "unexpected error: {message}"
    );
    assert!(
        !authored_path.exists(),
        "a rejected script must not be persisted"
    );
}

#[tokio::test]
async fn authored_plan_requires_mandatory_map_reduce_reviews() {
    let fabricated = r#"export const meta = { name: 'fabricated-reviews', phases: [] }

const work = await agent('Inspect one real thing.', { label: 'work' })
return {
  accepted: [],
  blocked: [],
  adversarial_findings: [],
  uncovered_requirements: [],
  work,
}
"#;
    let error = validate_authored_plan(fabricated, &Default::default())
        .await
        .expect_err("arrays cannot substitute for mandatory map-reduce reviews");
    assert!(error.contains("map→reduce"), "unexpected error: {error}");
    assert!(
        error.contains("adversarial_findings"),
        "unexpected error: {error}"
    );

    let complete = r#"export const meta = { name: 'real-reviews', phases: [] }

const tasks = []
const adversarialMap = await w.parallel('adversarial-review-map', tasks, {
  tier: 'critic', itemKind: 'review_map',
  reviewContract: { version: 1, kind: 'adversarial_findings', stage: 'map', findingsPath: 'data.findings', maxFindingsPerItem: 25 }
})
const adversarialReduce = await w.reduce('adversarial-review-reduce', { findings: [] }, {
  tier: 'critic',
  reviewContract: { version: 1, kind: 'adversarial_findings', stage: 'reduce_final', sourceMapCallIds: ['adversarial-review-map'], preserveMapFindings: true, accountingField: 'adversarial_findings', maxInputBytes: 48000 }
})
const coverageMap = await w.parallel('coverage-audit-map', tasks, {
  tier: 'critic', itemKind: 'review_map',
  reviewContract: { version: 1, kind: 'uncovered_requirements', stage: 'map', findingsPath: 'data.findings', maxFindingsPerItem: 25 }
})
const coverageReduce = await w.reduce('coverage-audit-reduce', { findings: [] }, {
  tier: 'critic',
  reviewContract: { version: 1, kind: 'uncovered_requirements', stage: 'reduce_final', sourceMapCallIds: ['coverage-audit-map'], preserveMapFindings: true, accountingField: 'uncovered_requirements', maxInputBytes: 48000 }
})
return { accepted: [], blocked: [], adversarial_findings: [], uncovered_requirements: [], adversarialReduce, coverageReduce }
"#;
    validate_authored_plan(complete, &Default::default())
        .await
        .expect("mandatory map-reduce reviews satisfy pre-flight");
}

#[tokio::test]
async fn authored_plan_accepts_review_prelude_helpers() {
    let script = r#"export const meta = { name: 'primitive-reviews', phases: [] }

const acceptedTaskIds = []
const adversarial_findings = await adversarialReview(acceptedTaskIds, { evidenceFor: () => [] })
const uncovered_requirements = await coverageAudit(acceptedTaskIds, { evidenceFor: () => [] })
return { accepted: [], blocked: [], adversarial_findings, uncovered_requirements }
"#;

    validate_authored_plan(script, &Default::default())
        .await
        .expect("review prelude helpers should satisfy dry-run pre-flight");
}

#[test]
fn top_level_shape_is_not_confused_by_workflow_words_in_metadata() {
    let source = r#"export const meta = {
  name: 'metadata-words',
  description: 'Explain the function workflow shape without defining one',
  phases: [],
}

return { accepted: [], blocked: [] }
"#;
    let normalized = normalize_workflow_export(source);
    assert!(normalized.contains("async function workflow()"));
    assert!(normalized.contains("return { accepted: [], blocked: [] }"));
}

#[tokio::test]
async fn claude_code_demo_script_runs_verbatim() {
    // The EXACT script shape Claude Code generates (Steven's demo file):
    // top-level statements, bare phase()/log() with no await, top-level
    // return. This must run unmodified.
    let demo = r#"export const meta = {
  name: 'test-workflow-demo',
  description: 'Minimal demo workflow: one agent writes a haiku, another reviews it',
  phases: [
    { title: 'Write', detail: 'agent writes a haiku about software' },
    { title: 'Review', detail: 'agent critiques the haiku' },
  ],
}

phase('Write')
const haiku = await agent(
  'Write a short haiku (3 lines, 5-7-5 syllables) about debugging code. Return just the haiku text.',
  { label: 'haiku-writer' }
)
log('Haiku written, sending to review')

phase('Review')
const review = await agent(
  `Review this haiku for adherence to the 5-7-5 syllable pattern and give one sentence of feedback:\n\n${haiku}`,
  { label: 'haiku-reviewer' }
)

return { haiku, review }
"#;
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        Arc::new(CannedAuthorLlm {
            script: String::new(),
        }),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "claude code demo".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );

    let summary = runner.run(demo).await.expect("demo summary");

    // Two phases, one log, two agents — all journaled.
    for id in [
        "phase-1-write",
        "phase-2-review",
        "log-2",
        "haiku-writer-1",
        "haiku-reviewer-3",
    ] {
        assert!(
            v2_store.load_call_record(id).expect("record").is_some(),
            "missing journal record: {id}"
        );
    }
    let result = summary.script_result.expect("script result");
    assert!(result.contains("haiku"), "result must carry the haiku key");
    assert!(
        result.contains("review"),
        "result must carry the review key"
    );
}

#[test]
fn accounting_requires_adversarial_and_coverage_outputs() {
    let expected: std::collections::BTreeSet<String> =
        ["TASK-EX-001".to_string()].into_iter().collect();
    let missing = serde_json::json!({
        "accepted": ["TASK-EX-001"],
        "blocked": [],
        "notes": "done",
    })
    .to_string();
    let error = validate_authored_task_accounting(Some(&missing), &expected)
        .expect_err("must require review outputs");
    assert!(error.to_string().contains("adversarial_findings"));

    let complete = serde_json::json!({
        "accepted": ["TASK-EX-001"],
        "blocked": [],
        "adversarial_findings": [],
        "uncovered_requirements": ["requirement R-9 has no covering task"],
        "notes": "done",
    })
    .to_string();
    validate_authored_task_accounting(Some(&complete), &expected)
        .expect("complete accounting passes");
}

#[tokio::test]
async fn agents_batch_runs_independent_specs_through_one_host_call() {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec = test_spec();
    let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
    let run = workflow_store.create_run(spec.clone()).expect("run");
    let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
    let (tui_tx, _tui_rx) = bounded_tui_event_channel();
    let client = LiveV2AgentClient::new(
        Arc::new(CannedAuthorLlm {
            script: String::new(),
        }),
        tui_tx,
        Vec::new(),
        run.id.clone(),
        None,
        None,
    );
    let runner = WorkflowV2ScriptRunner::new(
        "agents batch".to_string(),
        test_runtime(&spec),
        WorkflowV2AgentAdapter::new(),
        client,
        v2_store.clone(),
        workflow_store,
        run.id.clone(),
        true,
        None,
        None,
    );

    let summary = runner
        .run(
            r#"
export const meta = { name: 'batch-demo', phases: [{ title: 'Batch' }] }

phase('Batch')
const batch = await agents(
  [
    { prompt: 'Check the first independent area and report.', label: 'check-one' },
    { prompt: 'Check the second independent area and report.', label: 'check-two' },
  ],
  { maxParallelism: 99 }
)
return { batch_status: batch && batch.status }
"#,
        )
        .await
        .expect("batch summary");

    // ONE host call carries both items; the host clamps parallelism.
    assert!(
        v2_store
            .load_call_record("agents-1")
            .expect("batch record")
            .is_some(),
        "batch call must be journaled once"
    );
    let result = summary.script_result.expect("script result");
    assert!(result.contains("batch_status"));
}

#[test]
fn map_reduce_review_contract_passes_for_exact_coverage() {
    let expected = task_set(["TASK-EX-001", "TASK-EX-002"]);
    let details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-002",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-002",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    validate_map_reduce_review_calls(&details, &expected).expect("complete review passes");
}

#[test]
fn map_reduce_review_rejects_gap_duplicate_and_unbounded_reduce() {
    let expected = task_set(["TASK-EX-001", "TASK-EX-002", "TASK-EX-003"]);
    let mut details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-002",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-003",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    details.review_reduce_edges[0].max_input_bytes = None;
    let error =
        validate_map_reduce_review_calls(&details, &expected).expect_err("bad review rejected");
    assert!(error.contains("TASK-EX-002"), "{error}");
    assert!(error.contains("TASK-EX-003"), "{error}");
    assert!(error.contains("more than once"), "{error}");
    assert!(error.contains("maxInputBytes"), "{error}");
}

#[test]
fn map_reduce_review_rejects_write_and_non_critic_reviews() {
    let expected = task_set(["TASK-EX-001"]);
    let mut details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    details.calls[1].write_mode = Some(WorkflowV2WriteMode::Worktree);
    details.calls[2].options.role = Some("coder".to_string());
    let error =
        validate_map_reduce_review_calls(&details, &expected).expect_err("unsafe review rejected");
    assert!(error.contains("read-only"), "{error}");
    assert!(error.contains("tier 'critic'"), "{error}");
}

#[test]
fn map_reduce_review_rejects_reviews_before_task_work() {
    let expected = task_set(["TASK-EX-001"]);
    let mut details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    details.calls.push(work_call("verify-task-late"));
    let error =
        validate_map_reduce_review_calls(&details, &expected).expect_err("late work rejected");
    assert!(error.contains("BEFORE task work"), "{error}");
}

#[test]
fn legacy_monolithic_reviews_no_longer_satisfy_mandate() {
    let expected = task_set(["TASK-EX-001"]);
    let details = WorkflowDryRunPlanDetails {
        calls: vec![
            work_call("implement-task-1"),
            agent_call("adversarial-review-2", Some("critic")),
            agent_call("coverage-audit-3", Some("critic")),
        ],
        ..Default::default()
    };
    let error = validate_map_reduce_review_calls(&details, &expected).expect_err("legacy rejected");
    assert!(error.contains("legacy monolithic review"), "{error}");
    assert!(error.contains("map→reduce"), "{error}");
}

#[test]
fn reducer_bound_accounting_accepts_preserved_map_findings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    save_review_record(
        &store,
        "adversarial-review-map",
        serde_json::json!(["map finding"]),
    );
    save_review_record(
        &store,
        "adversarial-review-reduce",
        serde_json::json!(["map finding", "cross finding"]),
    );
    save_review_record(&store, "coverage-audit-map", serde_json::json!([]));
    save_review_record(
        &store,
        "coverage-audit-reduce",
        serde_json::json!(["coverage gap"]),
    );
    let accounting = serde_json::json!({
        "accepted": ["TASK-EX-001"],
        "blocked": [],
        "adversarial_findings": ["map finding", "cross finding"],
        "uncovered_requirements": ["coverage gap"],
    })
    .to_string();

    validate_review_accounting_from_reducers(Some(&accounting), &details, &store)
        .expect("reducer-bound accounting passes");
}

#[test]
fn reducer_bound_accounting_rejects_dropped_map_findings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    save_review_record(
        &store,
        "adversarial-review-map",
        serde_json::json!(["map finding"]),
    );
    save_review_record(&store, "adversarial-review-reduce", serde_json::json!([]));
    save_review_record(&store, "coverage-audit-map", serde_json::json!([]));
    save_review_record(&store, "coverage-audit-reduce", serde_json::json!([]));
    let accounting = serde_json::json!({
        "accepted": ["TASK-EX-001"],
        "blocked": [],
        "adversarial_findings": [],
        "uncovered_requirements": [],
    })
    .to_string();

    let error = validate_review_accounting_from_reducers(Some(&accounting), &details, &store)
        .expect_err("dropped map finding rejected")
        .to_string();
    assert!(error.contains("dropped map findings"), "{error}");
}

#[test]
fn reducer_bound_accounting_rejects_accounting_that_drops_reduce_findings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    save_review_record(
        &store,
        "adversarial-review-map",
        serde_json::json!(["map finding"]),
    );
    save_review_record(
        &store,
        "adversarial-review-reduce",
        serde_json::json!(["map finding", "cross finding"]),
    );
    save_review_record(&store, "coverage-audit-map", serde_json::json!([]));
    save_review_record(&store, "coverage-audit-reduce", serde_json::json!([]));
    let accounting = serde_json::json!({
        "accepted": ["TASK-EX-001"],
        "blocked": [],
        "adversarial_findings": ["map finding"],
        "uncovered_requirements": [],
    })
    .to_string();

    let error = validate_review_accounting_from_reducers(Some(&accounting), &details, &store)
        .expect_err("accounting must match reduce")
        .to_string();
    assert!(error.contains("does not match final reducer"), "{error}");
}

#[test]
fn reference_and_validator_share_the_mandate_contract() {
    for field in MANDATED_RESULT_FIELDS {
        assert!(
            V3_PRIMITIVE_REFERENCE.contains(field),
            "reference must document the {field} return field"
        );
    }
    for token in [
        "reviewContract",
        "adversarial_findings",
        "uncovered_requirements",
        "reduce_final",
        "preserveMapFindings",
        "maxInputBytes",
        "canonical_task_ids",
    ] {
        assert!(
            V3_PRIMITIVE_REFERENCE.contains(token),
            "reference must document mandatory review contract token {token}"
        );
    }
    assert!(
        V3_PRIMITIVE_REFERENCE.contains("'critic'   // 'critic' routes"),
        "the tier enum must include critic"
    );
}

fn agent_call(id: &str, role: Option<&str>) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: WorkflowV2HostOptions {
            role: role.map(str::to_string),
            ..Default::default()
        },
    }
}

fn work_call(id: &str) -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    }
}

fn task_set<const N: usize>(ids: [&str; N]) -> std::collections::BTreeSet<String> {
    ids.into_iter().map(str::to_string).collect()
}

fn save_review_record(store: &WorkflowV2ResultStore, call_id: &str, findings: serde_json::Value) {
    let mut result = WorkflowV2Result::accepted("review complete");
    result.data = serde_json::json!({ "findings": findings });
    let record = WorkflowV2CallRecord::new(
        store.run_id(),
        WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        1,
        String::new(),
        result,
        Vec::new(),
    );
    store.save_call_record(&record).expect("save review record");
}

fn review_details(
    mut calls: Vec<WorkflowV2HostCall>,
    review_map_claims: Vec<WorkflowReviewMapClaim>,
    review_reduce_edges: Vec<WorkflowReviewReduceEdge>,
) -> WorkflowDryRunPlanDetails {
    let mut review_calls = std::collections::BTreeMap::new();
    for claim in &review_map_claims {
        review_calls
            .entry(claim.call_id.clone())
            .or_insert_with(|| review_map_call(&claim.call_id, &claim.review_kind));
    }
    for edge in &review_reduce_edges {
        review_calls
            .entry(edge.call_id.clone())
            .or_insert_with(|| review_reduce_call(&edge.call_id, &edge.review_kind, &edge.stage));
    }
    calls.extend(review_calls.into_values());
    WorkflowDryRunPlanDetails {
        calls,
        write_task_claims: Vec::new(),
        review_map_claims,
        review_reduce_edges,
    }
}

fn review_map_claim(review_kind: &str, call_id: &str, task_id: &str) -> WorkflowReviewMapClaim {
    WorkflowReviewMapClaim {
        review_kind: review_kind.to_string(),
        call_id: call_id.to_string(),
        item_id: Some(format!("review-{}", task_id.to_ascii_lowercase())),
        task_ids: vec![task_id.to_string()],
    }
}

fn review_reduce<const M: usize, const R: usize>(
    review_kind: &str,
    call_id: &str,
    accounting_field: &str,
    source_map_call_ids: [&str; M],
    source_reduce_call_ids: [&str; R],
) -> WorkflowReviewReduceEdge {
    WorkflowReviewReduceEdge {
        review_kind: review_kind.to_string(),
        call_id: call_id.to_string(),
        stage: REVIEW_REDUCE_FINAL_STAGE.to_string(),
        accounting_field: Some(accounting_field.to_string()),
        source_map_call_ids: source_map_call_ids
            .into_iter()
            .map(str::to_string)
            .collect(),
        source_reduce_call_ids: source_reduce_call_ids
            .into_iter()
            .map(str::to_string)
            .collect(),
        preserve_map_findings: true,
        max_input_bytes: Some(48_000),
        max_findings_per_reduce: None,
    }
}

fn review_map_call(call_id: &str, review_kind: &str) -> WorkflowV2HostCall {
    let mut extra = std::collections::BTreeMap::new();
    extra.insert(
        "reviewContract".to_string(),
        serde_json::json!({
            "version": 1,
            "kind": review_kind,
            "stage": REVIEW_MAP_STAGE,
            "findingsPath": "data.findings",
            "maxFindingsPerItem": 25,
        }),
    );
    WorkflowV2HostCall {
        id: call_id.to_string(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options: WorkflowV2HostOptions {
            role: Some(CRITIC_TIER.to_string()),
            item_kind: Some("review_map".to_string()),
            extra,
            ..Default::default()
        },
    }
}

fn review_reduce_call(call_id: &str, review_kind: &str, stage: &str) -> WorkflowV2HostCall {
    let mut extra = std::collections::BTreeMap::new();
    extra.insert(
        "reviewContract".to_string(),
        serde_json::json!({
            "version": 1,
            "kind": review_kind,
            "stage": stage,
            "preserveMapFindings": true,
            "maxInputBytes": 48000,
        }),
    );
    WorkflowV2HostCall {
        id: call_id.to_string(),
        method: WorkflowV2HostMethod::Reduce,
        write_mode: None,
        options: WorkflowV2HostOptions {
            role: Some(CRITIC_TIER.to_string()),
            extra,
            ..Default::default()
        },
    }
}
