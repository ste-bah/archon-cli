use super::*;

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
