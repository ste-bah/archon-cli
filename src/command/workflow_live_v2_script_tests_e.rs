
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
        let authored_path = workflow_store
            .run_dir(&run.id)
            .join("authored-workflow.js");

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
    async fn authored_plan_requires_both_mandatory_review_agents() {
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
            .expect_err("arrays cannot substitute for mandatory review agents");
        assert!(error.contains("adversarial-review"), "unexpected error: {error}");

        let complete = r#"export const meta = { name: 'real-reviews', phases: [] }

await agent('Inspect one real thing.', { label: 'work' })
await agent('Falsify the claims.', { label: 'adversarial-review', tier: 'critic' })
await agent('Audit source coverage.', { label: 'coverage-audit' })
return { accepted: [], blocked: [], adversarial_findings: [], uncovered_requirements: [] }
"#;
        validate_authored_plan(complete, &Default::default())
            .await
            .expect("both mandatory agents satisfy pre-flight");
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
        assert!(result.contains("review"), "result must carry the review key");
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
    fn missing_mandates_are_reported_together() {
        let planned = vec![agent_call("implement-something-1", None)];
        let error = validate_mandatory_review_calls(&planned).expect_err("both missing");
        for (label, _, _) in MANDATED_REVIEWS {
            assert!(error.contains(label), "{error}");
        }
        assert!(error.contains("fix EVERY one"), "{error}");
    }

    #[test]
    fn complete_mandates_pass_and_partial_reports_only_the_missing_one() {
        let complete = vec![
            work_call("implement-task-1"),
            agent_call("adversarial-review-7", Some("critic")),
            agent_call("coverage-audit-8", None),
        ];
        validate_mandatory_review_calls(&complete).expect("complete plan passes");

        let partial = vec![
            work_call("implement-task-1"),
            agent_call("adversarial-review-7", Some("critic")),
        ];
        let error = validate_mandatory_review_calls(&partial).expect_err("coverage missing");
        assert!(error.contains("coverage-audit"), "{error}");
        assert!(!error.contains("`adversarial-review` is missing"), "{error}");
    }

    #[test]
    fn extended_labels_never_satisfy_a_mandate() {
        let sneaky = vec![
            work_call("implement-task-1"),
            agent_call("adversarial-review-skip-2", Some("critic")),
            agent_call("coverage-audit-prep-3", None),
        ];
        let error = validate_mandatory_review_calls(&sneaky).expect_err("extended labels");
        assert!(error.contains("extended labels do not count"), "{error}");
        assert!(error.contains("adversarial-review-skip-2"), "{error}");
        assert!(error.contains("coverage-audit-prep-3"), "{error}");
    }

    #[test]
    fn wrong_call_kind_is_named_not_reported_as_omitted() {
        let mut batch = agent_call("adversarial-review-4", Some("critic"));
        batch.method = WorkflowV2HostMethod::Parallel;
        let mut written = agent_call("coverage-audit-5", None);
        written.method = WorkflowV2HostMethod::Fanout;
        let planned = vec![work_call("implement-task-1"), batch, written];
        let error = validate_mandatory_review_calls(&planned).expect_err("wrong kinds");
        assert!(error.contains("SEPARATE top-level read-only agent()"), "{error}");
        assert!(!error.contains("is missing"), "{error}");
    }

    #[test]
    fn adversarial_review_requires_the_critic_tier() {
        let planned = vec![
            work_call("implement-task-1"),
            agent_call("adversarial-review-2", Some("coder")),
            agent_call("coverage-audit-3", None),
        ];
        let error = validate_mandatory_review_calls(&planned).expect_err("tier enforced");
        assert!(error.contains("tier 'critic'"), "{error}");
    }

    #[test]
    fn reviews_before_task_work_are_rejected() {
        let planned = vec![
            agent_call("adversarial-review-1", Some("critic")),
            agent_call("coverage-audit-2", None),
            work_call("implement-task-3"),
        ];
        let error = validate_mandatory_review_calls(&planned).expect_err("position enforced");
        assert!(error.contains("BEFORE task work"), "{error}");
    }

    #[test]
    fn reviews_before_final_read_only_verification_are_rejected() {
        let planned = vec![
            work_call("implement-task-1"),
            agent_call("adversarial-review-2", Some("critic")),
            agent_call("coverage-audit-3", None),
            agent_call("verify-task-4", None),
        ];
        let error = validate_mandatory_review_calls(&planned).expect_err("reviews must be last");
        assert!(error.contains("BEFORE task work"), "{error}");
        assert!(error.contains("agent"), "{error}");
    }

    #[test]
    fn reference_and_validator_share_the_mandate_contract() {
        for (label, _, requires_critic) in MANDATED_REVIEWS {
            assert!(
                V3_PRIMITIVE_REFERENCE.contains(&format!("label: '{label}'")),
                "reference must show the mandated label {label}"
            );
            if requires_critic {
                assert!(
                    V3_PRIMITIVE_REFERENCE.contains(&format!("label: '{label}', tier: 'critic'")),
                    "reference must show the critic tier on {label}"
                );
            }
        }
        for field in MANDATED_RESULT_FIELDS {
            assert!(
                V3_PRIMITIVE_REFERENCE.contains(field),
                "reference must document the {field} return field"
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
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        }
    }
