
    #[tokio::test]
    async fn prose_target_files_fail_fast_in_the_script() {
        let temp = tempfile::tempdir().expect("tempdir");
        let spec = test_spec();
        let workflow_store = WorkflowStore::new(temp.path().join("workflows"));
        let run = workflow_store.create_run(spec.clone()).expect("run");
        let v2_store = WorkflowV2ResultStore::new(workflow_store.run_dir(&run.id).join("v2"));
        let (tui_tx, _tui_rx) = bounded_tui_event_channel();
        let client = LiveV2AgentClient::new(
            Arc::new(PanicLlm),
            tui_tx,
            Vec::new(),
            run.id.clone(),
            None,
            None,
        );
        let runner = WorkflowV2ScriptRunner::new(
            "prose targets".to_string(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            client,
            v2_store,
            workflow_store,
            run.id.clone(),
            true,
            None,
            None,
        );

        let summary = runner
            .run(
                r#"
export const meta = { name: 'prose-targets', phases: [{ title: 'One' }] }
const done = await agent('Implement the thing.', {
  label: 'implement-thing',
  write: true,
  taskIds: ['TASK-EX-001'],
  targetFiles: ['project task artifacts/context under /some/dir'],
})
return { accepted: [], blocked: [], adversarial_findings: [], uncovered_requirements: [], notes: 'n' }
"#,
            )
            .await
            .expect("summary");

        assert_eq!(summary.status, WorkflowV2Status::Failed);
        assert_eq!(
            summary.failed_call.as_deref(),
            Some("workflow.js"),
            "prose targetFiles must fail in the script layer, before any host call"
        );
        let next = summary.next_action.unwrap_or_default();
        assert!(
            next.contains("workflow.js"),
            "failure must be attributed to the script: {next}"
        );
    }

    #[tokio::test]
    async fn dry_run_host_guard_rejects_prose_targets_even_when_caught() {
        // Raw w.fanout with prose targets inside try/catch: the script-side
        // sugar is bypassed AND the throw is swallowed — the HOST recorder
        // still fails the dry run (policy errors are authoritative).
        let script = r#"
async function workflow(w) {
  try {
    await w.fanout("write-1", [{ item_id: "i1", target_files: ["project task artifacts under /some/dir"], task: "x" }], { write: "worktree", task: "x" });
  } catch (err) {}
  await w.checkpoint("done", { task: "record" });
  return {};
}
"#;
        let error = dry_run_workflow_plan(script, None)
            .await
            .expect_err("prose target must fail the dry run");
        assert!(
            error.to_string().contains("literal repo-relative file paths"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn dry_run_accepts_extensionless_targets_and_rejects_globs() {
        let ok = r#"
async function workflow(w) {
  await w.fanout("write-1", [{ item_id: "i1", target_files: ["Makefile"], task: "x" }], { write: "worktree", task: "x" });
  return {};
}
"#;
        dry_run_workflow_plan(ok, None)
            .await
            .expect("Makefile is a valid literal target");

        let glob = r#"
async function workflow(w) {
  await w.fanout("write-1", [{ item_id: "i1", target_files: ["src/**/*.rs"], task: "x" }], { write: "worktree", task: "x" });
  return {};
}
"#;
        let error = dry_run_workflow_plan(glob, None)
            .await
            .expect_err("globs must be rejected");
        assert!(error.to_string().contains("glob"), "{error}");
    }

    #[tokio::test]
    async fn preflight_names_tasks_missing_from_write_coverage() {
        let expected: std::collections::BTreeSet<String> =
            ["TASK-EX-001".to_string(), "TASK-EX-002".to_string()]
                .into_iter()
                .collect();
        // Write coverage for 001 only; 002 must be named as missing.
        let script = r#"
async function workflow(w) {
  await w.fanout("write-1", [{ item_id: "i1", canonical_task_ids: ["TASK-EX-001"], target_files: ["src/lib.rs"], task: "x" }], { write: "worktree", task: "x" });
  await w.agent("adversarial-review-1", { tier: "critic", task: "falsify" });
  await w.agent("coverage-audit-2", { task: "audit" });
  return {};
}
"#;
        let error = validate_authored_plan(script, &expected)
            .await
            .expect_err("missing write coverage must fail");
        assert!(error.contains("TASK-EX-002"), "{error}");
        assert!(!error.contains("NO write coverage") || !error.split("NO write coverage").nth(1).unwrap_or("").contains("TASK-EX-001"), "{error}");

        let umbrella = r#"
async function workflow(w) {
  await w.fanout("write-1", [{ item_id: "i1", canonical_task_ids: ["TASK-EX-001", "TASK-EX-002"], target_files: ["src/lib.rs"], task: "x" }], { write: "worktree", task: "x" });
  await w.agent("adversarial-review-1", { tier: "critic", task: "falsify" });
  await w.agent("coverage-audit-2", { task: "audit" });
  return {};
}
"#;
        let error = validate_authored_plan(umbrella, &expected)
            .await
            .expect_err("small universes must reject umbrella task claims");
        assert!(error.contains("umbrella id-stuffing"), "{error}");

        let complete = r#"
async function workflow(w) {
  await w.fanout("write-1", [{ item_id: "i1", canonical_task_ids: ["TASK-EX-001"], target_files: ["src/lib.rs"], task: "x" }], { write: "worktree", task: "x" });
  await w.fanout("write-2", [{ item_id: "i2", canonical_task_ids: ["TASK-EX-002"], target_files: ["src/main.rs"], task: "y" }], { write: "worktree", task: "y" });
  await w.agent("adversarial-review-1", { tier: "critic", task: "falsify" });
  await w.agent("coverage-audit-2", { task: "audit" });
  return {};
}
"#;
        validate_authored_plan(complete, &expected)
            .await
            .expect("full write coverage passes");
    }

    #[tokio::test]
    async fn verify_class_agents_route_through_the_verification_wave_machinery() {
        // verify:true / focusedTests reroute to a verification-wave-prefixed
        // parallel call — the HOST prefix that grants command execution and
        // attaches the focused-verification guards. Plain readers and the
        // critic-tier adversarial review stay plain agent calls (no shell).
        let script = r#"
export const meta = { name: 'verify-route-demo', phases: [{ title: 'One' }] }
const plain = agent('Read something.', { label: 'plain-reader' })
const verifier = agent('Run the tests.', { label: 'task-verifier', verify: true, taskIds: ['TASK-EX-001'], focusedTests: ['cargo test exact_name'] })
const critic = agent('Falsify claims by inspection.', { label: 'adversarial-review', tier: 'critic' })
await Promise.all([plain, verifier, critic])
return { accepted: [], blocked: [], adversarial_findings: [], uncovered_requirements: [], notes: 'n' }
"#;
        let (calls, _) = dry_run_workflow_plan_details(script, None)
            .await
            .expect("plan");
        let verifier_call = calls
            .iter()
            .find(|call| call.id.starts_with("verification-wave-task-verifier"))
            .expect("verifier must become a verification-wave call");
        assert_eq!(verifier_call.method, WorkflowV2HostMethod::Parallel);
        let critic_call = calls
            .iter()
            .find(|call| call.id.starts_with("adversarial-review"))
            .expect("critic call");
        assert_eq!(critic_call.method, WorkflowV2HostMethod::Agent);
        assert!(critic_call.options.item_kind.is_none());
        let plain_call = calls
            .iter()
            .find(|call| call.id.starts_with("plain-reader"))
            .expect("plain call");
        assert_eq!(plain_call.method, WorkflowV2HostMethod::Agent);
    }
