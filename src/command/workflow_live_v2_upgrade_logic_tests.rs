//! Issue 361, end to end through the script runner: a logic change re-runs
//! only its capability, an upgrade without one reuses everything, and a
//! verdict recorded before logic versions never answers for a check.
use super::*;
use crate::command::workflow_host_command_logic::{
    CAPABILITY_LOGIC, LOGIC_DIGEST_STAMP, LOGIC_VERSION_STAMP,
};

/// A binary from before logic versions: the same keys, no stamp, no check.
struct Unversioned(CatalogHost);

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for Unversioned {
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        self.0.call_identity(request)
    }
    fn record_is_reusable(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        self.0.record_is_reusable(record)
    }
    async fn execute(
        &self,
        request: archon_workflow::HostCommandRequest,
        generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        self.0.execute(request, generation).await
    }
}

/// One fixed run that successive builds resume.
struct Run {
    _temp: tempfile::TempDir,
    spec: WorkflowSpec,
    store: WorkflowStore,
    run_id: String,
    v2: WorkflowV2ResultStore,
    context: crate::command::workflow_host_command_catalog::HostCommandResolutionContext,
    launch: archon_workflow::CommandCapabilityCatalog,
}

impl Run {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let spec = test_spec();
        let store = WorkflowStore::new(temp.path().join("workflows"));
        let run = store.create_run(spec.clone()).unwrap();
        let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
        let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
        Self {
            _temp: temp,
            spec,
            store,
            run_id: run.id,
            v2,
            context,
            launch: launch_catalog(),
        }
    }

    /// The launch build.
    fn launched(&self) -> CatalogHost {
        CatalogHost {
            keys: FixedHostCommandExecutor::new(
                self.launch.clone(),
                self.context.clone(),
                self.store.run_dir(&self.run_id),
            ),
            calls: AtomicUsize::new(0),
        }
    }

    /// A later build: a freeze-skeleton limit changed (a binary that is not
    /// the launch one), and each of `bumps` runs another logic version.
    fn upgraded(&self, bumps: &[(&str, u32)]) -> Arc<CatalogHost> {
        self.rebuilt(bumps, &[])
    }

    /// As [`Self::upgraded`], where each of `sources` hashes to other source.
    fn rebuilt(&self, bumps: &[(&str, u32)], sources: &[&str]) -> Arc<CatalogHost> {
        let mut keys = FixedHostCommandExecutor::new(
            changed(&self.launch, "freeze-skeleton", "timeout"),
            self.context.clone(),
            self.store.run_dir(&self.run_id),
        )
        .with_launch_catalog(self.launch.clone());
        for (id, version) in bumps {
            keys = keys.with_logic_version(id, Some(*version));
        }
        for id in sources {
            keys = keys.with_logic_digest(id, "other-source");
        }
        Arc::new(CatalogHost {
            keys,
            calls: AtomicUsize::new(0),
        })
    }

    /// Runs `script` on `executor`: (executed, reused).
    async fn run(
        &self,
        executor: Arc<dyn WorkflowHostCommandExecutor>,
        script: &str,
    ) -> (usize, usize) {
        let (sink, _rx) = default_workflow_ui_sink();
        let summary = WorkflowV2ScriptRunner::new(
            "logic upgrade".into(),
            test_runtime(&self.spec),
            WorkflowV2AgentAdapter::new(),
            LiveV2AgentClient::new(
                Arc::new(PanicLlm),
                sink,
                Vec::new(),
                self.run_id.clone(),
                None,
                None,
            ),
            self.v2.clone(),
            self.store.clone(),
            self.run_id.clone(),
            true,
            None,
            None,
        )
        .with_raw_outcomes(true)
        .with_host_command_executor(executor)
        .run(script)
        .await
        .unwrap();
        (summary.executed, summary.reused)
    }

    /// The logic digest each recorded outcome of `command` carries.
    fn digests(&self, command: &str) -> Vec<Option<String>> {
        let records = self.v2.load_call_records().unwrap().into_iter();
        records
            .filter(|record| {
                let request = record.call.options.host_command.as_ref();
                request.is_some_and(|request| request.command_id == command)
            })
            .map(|record| {
                let digest = record.result.data.get(LOGIC_DIGEST_STAMP);
                digest.and_then(|v| v.as_str()).map(str::to_string)
            })
            .collect()
    }

    /// The logic version each recorded outcome of `command` carries.
    fn stamps(&self, command: &str) -> Vec<Option<u64>> {
        self.v2
            .load_call_records()
            .unwrap()
            .into_iter()
            .filter(|record| {
                record
                    .call
                    .options
                    .host_command
                    .as_ref()
                    .is_some_and(|request| request.command_id == command)
            })
            .map(|record| {
                record
                    .result
                    .data
                    .get(LOGIC_VERSION_STAMP)
                    .and_then(|v| v.as_u64())
            })
            .collect()
    }
}

/// A landing, then the checks before Completed.
const SCRIPT: &str = r#"async function workflow(w) {
    await w.hostCommand("freeze-skeleton", {stdin: "candidate"});
    await w.hostCommand("verify-frozen-acceptance", {});
    await w.hostCommand("task-set-lint", {});
    await w.hostCommand("requirements-trace", {});
}"#;

#[tokio::test]
async fn logic_361_a_logic_change_reruns_only_its_capability() {
    let run = Run::new();
    assert_eq!(run.run(Arc::new(run.launched()), SCRIPT).await, (4, 0));
    assert_eq!(run.stamps("freeze-skeleton"), vec![Some(1)]);
    let stricter = run.upgraded(&[("freeze-skeleton", 2)]);
    assert_eq!(run.run(stricter.clone(), SCRIPT).await, (1, 3));
    assert_eq!(stricter.calls.load(Ordering::SeqCst), 1);
    // The new verdict is keyed and stamped by the new logic, and holds.
    assert!(run.stamps("freeze-skeleton").contains(&Some(2)));
    assert_eq!(
        run.run(run.upgraded(&[("freeze-skeleton", 2)]), SCRIPT)
            .await,
        (0, 4)
    );
    // A stricter set lint re-runs the lint alone.
    let lint = run.upgraded(&[("freeze-skeleton", 2), ("task-set-lint", 2)]);
    assert_eq!(run.run(lint.clone(), SCRIPT).await, (1, 3));
    assert!(run.stamps("task-set-lint").contains(&Some(2)));
}

#[tokio::test]
async fn logic_361_an_upgrade_that_changes_no_logic_reuses_everything() {
    let run = Run::new();
    assert_eq!(run.run(Arc::new(run.launched()), SCRIPT).await, (4, 0));
    let script = format!("{SCRIPT}\n// next harness");
    let upgraded = run.upgraded(&[]);
    assert_eq!(run.run(upgraded.clone(), &script).await, (0, 4));
    assert_eq!(upgraded.calls.load(Ordering::SeqCst), 0);
}

/// Records written before logic versions: the landing reuses, every check
/// runs again once and is stamped, and then reuses too.
#[tokio::test]
async fn logic_361_unversioned_checks_run_again_and_an_unversioned_landing_reuses() {
    let run = Run::new();
    assert_eq!(
        run.run(Arc::new(Unversioned(run.launched())), SCRIPT).await,
        (4, 0)
    );
    assert_eq!(run.stamps("verify-frozen-acceptance"), vec![None]);
    let upgraded = run.upgraded(&[]);
    assert_eq!(run.run(upgraded.clone(), SCRIPT).await, (3, 1));
    assert_eq!(upgraded.calls.load(Ordering::SeqCst), 3);
    for check in [
        "verify-frozen-acceptance",
        "task-set-lint",
        "requirements-trace",
    ] {
        assert_eq!(run.stamps(check), vec![Some(1)], "{check}");
    }
    assert_eq!(run.stamps("freeze-skeleton"), vec![None], "landing reused");
    assert_eq!(run.run(run.upgraded(&[]), SCRIPT).await, (0, 4));
}

#[tokio::test]
async fn logic_361_an_unversioned_landing_reruns_after_its_first_bump() {
    let run = Run::new();
    assert_eq!(
        run.run(Arc::new(Unversioned(run.launched())), SCRIPT).await,
        (4, 0)
    );
    let stricter = run.upgraded(&[("freeze-skeleton", 2)]);
    assert_eq!(run.run(stricter, SCRIPT).await, (4, 0));
}

/// A check asked twice: the first answer is history once the second is
/// recorded. Recorded before logic versions, it is not replayed from
/// history either; stamped, it is.
#[tokio::test]
async fn logic_361_unversioned_check_history_is_not_replayed() {
    let run = Run::new();
    let script = r#"async function workflow(w) {
        await w.hostCommand("verify-frozen-acceptance", {});
        await w.hostCommand("verify-frozen-acceptance", {});
    }"#;
    assert_eq!(
        run.run(Arc::new(Unversioned(run.launched())), script).await,
        (2, 0)
    );
    let upgraded = run.upgraded(&[]);
    assert_eq!(run.run(upgraded.clone(), script).await, (2, 0));
    assert_eq!(upgraded.calls.load(Ordering::SeqCst), 2);
    assert_eq!(run.run(run.upgraded(&[]), script).await, (0, 2));
}

/// The backstop: a later binary whose source of a cheap check differs runs
/// that check again even at the same version; task-set-lint and the
/// landings keep version-only reuse.
#[tokio::test]
async fn logic_361_other_source_reruns_only_the_cheap_checks() {
    let run = Run::new();
    assert_eq!(run.run(Arc::new(run.launched()), SCRIPT).await, (4, 0));
    let pinned = |id: &str| {
        let logic = CAPABILITY_LOGIC
            .iter()
            .find(|logic| logic.id == id)
            .unwrap();
        Some(logic.sources_digest.to_string())
    };
    assert_eq!(
        run.digests("requirements-trace"),
        vec![pinned("requirements-trace")]
    );
    let all = [
        "freeze-skeleton",
        "verify-frozen-acceptance",
        "task-set-lint",
        "requirements-trace",
    ];
    let rebuilt = run.rebuilt(&[], &all);
    assert_eq!(run.run(rebuilt.clone(), SCRIPT).await, (2, 2));
    assert_eq!(rebuilt.calls.load(Ordering::SeqCst), 2);
    // Run again and stamped by the new source, they now replay on it.
    assert!(
        run.digests("verify-frozen-acceptance")
            .contains(&Some("other-source".into()))
    );
    assert_eq!(run.digests("task-set-lint"), vec![pinned("task-set-lint")]);
    assert_eq!(run.run(run.rebuilt(&[], &all), SCRIPT).await, (0, 4));
    // Only one cheap check's source changed: only it runs.
    let one = run.rebuilt(
        &[],
        &["freeze-skeleton", "task-set-lint", "requirements-trace"],
    );
    assert_eq!(run.run(one, SCRIPT).await, (1, 3));
}
