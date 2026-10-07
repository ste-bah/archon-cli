use super::*;

async fn author_reuse_after_upgrade(suffix: &str) {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let (sink, _rx) = default_workflow_ui_sink();
    let llm = Arc::new(RawOutcomeLlm {
        calls: AtomicUsize::new(0),
        requests: Default::default(),
        content: "candidate",
        stop_reason: "end_turn",
    });
    let executor = Arc::new(FakeHostCommandExecutor {
        calls: AtomicUsize::new(0),
    });
    let runner = || {
        WorkflowV2ScriptRunner::new(
            "upgrade reuse".into(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            LiveV2AgentClient::new(
                llm.clone(),
                sink.clone(),
                Vec::new(),
                run.id.clone(),
                None,
                None,
            )
            .with_fixed_raw_tool_policy(vec!["Read".into()]),
            v2.clone(),
            store.clone(),
            run.id.clone(),
            true,
            None,
            None,
        )
        .with_raw_outcomes(true)
        .with_host_command_executor(executor.clone())
    };
    let script = r#"async function workflow(w) {
      return await w.agent("acceptance-author-1", {task: "Author candidate", tier: "planner", resultMode: "rawOutcome"});
    }"#;
    assert_eq!(runner().run(script).await.unwrap().executed, 1);
    let upgraded = format!("{script}\n{suffix}");
    let summary = runner().run(&upgraded).await.unwrap();
    assert_eq!(
        summary.reused, 1,
        "unchanged prompt on changed script must reuse"
    );
    assert_eq!(summary.executed, 0);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1);
    let changed_prompt = upgraded.replace("Author candidate", "Author different candidate");
    let summary = runner().run(&changed_prompt).await.unwrap();
    assert_eq!(summary.reused, 0, "changed prompt must execute");
    assert_eq!(summary.executed, 1);
    assert_eq!(llm.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn upgrade_358_author_reuses_after_script_comment_change() {
    author_reuse_after_upgrade("// upgraded harness").await;
}
#[tokio::test]
async fn upgrade_358_author_reuses_after_helper_change() {
    author_reuse_after_upgrade("function helper() { return 2; }").await;
}
#[tokio::test]
async fn upgrade_358_author_reuses_after_unrelated_stage_change() {
    author_reuse_after_upgrade("async function unrelated(w) { return w.checkpoint('other', {}); }")
        .await;
}

use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, WorkflowHostCommandExecutor,
};

struct CatalogHost {
    keys: FixedHostCommandExecutor,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for CatalogHost {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        self.keys.call_identity(request)
    }
    fn record_is_reusable(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        let outcome: archon_workflow::HostCommandResult =
            serde_json::from_value(record.result.data.clone())?;
        Ok(outcome.reusable()
            && crate::command::workflow_host_command_occurrence::record_identity_matches(
                record,
                &self.call_identity(record.call.options.host_command.as_ref().unwrap())?,
            ))
    }
    fn outcome_limits_hold(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        self.keys.outcome_limits_hold(record)
    }
    fn limits_fingerprint(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<Option<serde_json::Value>> {
        self.keys.limits_fingerprint(request)
    }
    fn logic_version(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<Option<u32>> {
        self.keys.logic_version(request)
    }
    fn outcome_logic_holds(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        self.keys.outcome_logic_holds(record)
    }
    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
        generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let stdin = request.stdin.clone().unwrap_or_default();
        // Simulated process result; identity and reuse use the real executor.
        let mut outcome = FakeHostCommandExecutor {
            calls: AtomicUsize::new(0),
        }
        .execute(request, generation)
        .await?;
        if stdin == "slow" || stdin == "loud" {
            // Cut short by a limit: nothing landed.
            outcome.exit_code = None;
            outcome.timed_out = stdin == "slow";
            outcome.stdout_truncated = stdin == "loud";
            outcome.publication_receipt = None;
            outcome.postcondition = None;
        }
        Ok(outcome)
    }
}

fn launch_catalog() -> archon_workflow::CommandCapabilityCatalog {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/crates/archon-workflow/tests/fixtures/fixed-command-catalog-v1.json"
    )))
    .unwrap()
}

/// `launch` with one change to the capability `command`.
fn changed(
    launch: &archon_workflow::CommandCapabilityCatalog,
    command: &str,
    change: &str,
) -> archon_workflow::CommandCapabilityCatalog {
    let mut catalog = launch.clone();
    let capability = catalog.capabilities.get_mut(command).unwrap();
    match change {
        "none" => {}
        "timeout" => capability.timeout_secs += 600,
        "timeout_again" => capability.timeout_secs += 1200,
        "stdout_limit" => capability.max_stdout_bytes += 1,
        "schema" => {
            catalog.schema_version += 1;
            catalog.capabilities.get_mut(command).unwrap().timeout_secs += 600;
        }
        "argv" => capability.argv_template.push("--new-bound".into()),
        "environment" => {
            capability.environment_profile = archon_workflow::EnvironmentProfileId::FreezeProvider
        }
        _ => unreachable!(),
    }
    catalog.recompute_digest().unwrap();
    catalog
}

async fn catalog_upgrade(change: &str) {
    let temp = tempfile::tempdir().unwrap();
    let spec = test_spec();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store.create_run(spec.clone()).unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
    let launch = launch_catalog();
    let mut canonical = launch.clone();
    canonical.recompute_digest().unwrap();
    assert_eq!(canonical.digest, launch.digest, "exact live catalog shape");
    let first = Arc::new(CatalogHost {
        keys: FixedHostCommandExecutor::new(
            launch.clone(),
            context.clone(),
            store.run_dir(&run.id),
        ),
        calls: AtomicUsize::new(0),
    });
    let catalog = changed(&launch, "freeze-skeleton", change);
    // What the freeze does changed; a limit alone does not re-key it.
    let rerun = matches!(change, "argv" | "environment");
    let second = Arc::new(CatalogHost {
        keys: FixedHostCommandExecutor::new(catalog, context, store.run_dir(&run.id))
            .with_launch_catalog(launch),
        calls: AtomicUsize::new(0),
    });
    let unchanged =
        archon_workflow::HostCommandRequest::new("verify-frozen-acceptance", None).unwrap();
    assert_eq!(
        first.call_identity(&unchanged).unwrap(),
        second.call_identity(&unchanged).unwrap(),
        "unaffected commands retain legacy store keys"
    );
    let (sink, _rx) = default_workflow_ui_sink();
    let llm = Arc::new(RawOutcomeLlm {
        calls: AtomicUsize::new(0),
        requests: Default::default(),
        content: "candidate",
        stop_reason: "end_turn",
    });
    let runner = |executor: Arc<CatalogHost>| {
        WorkflowV2ScriptRunner::new(
            "catalog upgrade".into(),
            test_runtime(&spec),
            WorkflowV2AgentAdapter::new(),
            LiveV2AgentClient::new(
                llm.clone(),
                sink.clone(),
                Vec::new(),
                run.id.clone(),
                None,
                None,
            )
            .with_fixed_raw_tool_policy(vec!["Read".into()]),
            v2.clone(),
            store.clone(),
            run.id.clone(),
            true,
            None,
            None,
        )
        .with_raw_outcomes(true)
        .with_host_command_executor(executor)
    };
    let script = r#"async function workflow(w) {
        const a = await w.agent("acceptance-author-1", {task: "Author candidate", tier: "planner", resultMode: "rawOutcome"});
        await w.hostCommand("freeze-skeleton", {stdin: a.content});
        await w.hostCommand("verify-frozen-acceptance", {});
    }"#;
    assert_eq!(runner(first.clone()).run(script).await.unwrap().executed, 3);
    let summary = runner(second.clone())
        .run(&format!("{script}\n// updated harness"))
        .await
        .unwrap();
    assert_eq!(
        summary.executed,
        usize::from(rerun),
        "{change}: only a freeze whose meaning changed re-executes"
    );
    assert_eq!(summary.reused, 3 - usize::from(rerun), "{change}");
    assert_eq!(second.calls.load(Ordering::SeqCst), usize::from(rerun));
    assert_eq!(llm.calls.load(Ordering::SeqCst), 1);
    let summary = runner(second.clone())
        .run(&format!("{script}\n// next binary"))
        .await
        .unwrap();
    assert_eq!(summary.executed, 0);
    assert_eq!(summary.reused, 3);
    let changed_stdin = script.replace("stdin: a.content", "stdin: a.content + 'changed'");
    let summary = runner(second.clone()).run(&changed_stdin).await.unwrap();
    assert_eq!(summary.executed, 1, "new stdin cannot trust prior freeze");
    assert_eq!(summary.reused, 2);
}

#[tokio::test]
async fn upgrade_358_catalog_timeout_keeps_completed_freeze() {
    catalog_upgrade("timeout").await;
}
#[tokio::test]
async fn upgrade_358_catalog_output_limit_keeps_completed_freeze() {
    catalog_upgrade("stdout_limit").await;
}
#[tokio::test]
async fn upgrade_358_catalog_limit_only_schema_bump_keeps_completed_freeze() {
    catalog_upgrade("schema").await;
}
#[tokio::test]
async fn upgrade_358_catalog_argv_reruns_freeze_only() {
    catalog_upgrade("argv").await;
}
#[tokio::test]
async fn upgrade_358_catalog_environment_reruns_freeze_only() {
    catalog_upgrade("environment").await;
}

#[path = "workflow_live_v2_upgrade_limit_tests.rs"]
mod limit_tests;
#[path = "workflow_live_v2_upgrade_logic_tests.rs"]
mod logic_tests;
