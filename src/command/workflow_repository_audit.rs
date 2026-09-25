//! Composition of the mandatory audit with provider-neutral workflow execution.
use super::*;
use archon_workflow::WorkflowResult;
use archon_workflow::repository_audit::runtime::{AuditRuntime, Snapshot};
use std::collections::BTreeSet;

impl WorkflowV2ScriptRunner {
    pub(super) async fn initialize_repository_audit(&mut self) -> WorkflowResult<()> {
        if self.client.audit.is_some() {
            return Ok(());
        }
        let policy = self.client.audit_policy().unwrap_or_else(|| {
            use archon_workflow::repository_audit::budget::{AuditPolicy, Limit};
            AuditPolicy {
                attempt_timeout_secs: Limit::Finite(u64::from(
                    self.runtime.generated_config.host_call_timeout_secs,
                )),
                total_time_secs: Limit::Unlimited,
                unexpected_change_refreshes: Limit::Finite(3),
            }
        });
        let audit =
            AuditRuntime::initialize(self.workflow_store.clone(), self.run_id.clone(), policy)?;
        audit.update(|state| {
            if state.policy_provenance.is_none() {
                state.policy_provenance = Some(self.client.audit_provenance().unwrap_or_else(|| serde_json::json!({
                    "version":1,"source":"host runtime defaults","attempt_timeout_source":"workflow.generated.host_call_timeout_secs"})));
            }
            Ok(())
        })?;
        let paths = self
            .task_universe
            .as_ref()
            .map(|universe| {
                universe
                    .tasks
                    .iter()
                    .flat_map(|t| {
                        t.deliverable_contracts
                            .iter()
                            .map(|c| c.artifact_path.clone())
                    })
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        let repository_paths = declaration_paths::repository_paths(
            paths,
            self.runtime.target_repository_root.as_deref(),
            &self.v2_store,
        )?;
        audit.update(|s| {
            s.declared_paths.extend(repository_paths);
            Ok(())
        })?;
        self.client = self.client.with_audit(audit.clone());
        if let Some(root) = &self.runtime.target_repository_root {
            // Issue-26: state from before this rule may still carry an
            // obligation for a gitignored deliverable; reclaim it before the
            // initial assessment so the run stops re-dispatching that task.
            audit.reclaim_ignored(std::path::Path::new(root))?;
            let paths = audit
                .state()?
                .declared_paths
                .into_iter()
                .collect::<Vec<_>>();
            let snapshot = Snapshot::capture(std::path::Path::new(root), &paths, &self.v2_store)?;
            let assessor = AuditDispatch(self.client.for_audit());
            audit
                .assess(&snapshot, &paths, "initial", &assessor)
                .await?;
        } else {
            let root = self.v2_store.root().join("repository-audit/no-repository");
            std::fs::create_dir_all(&root).map_err(|e| WorkflowError::Io {
                path: root.clone(),
                source: e,
            })?;
            let snapshot = Snapshot {
                identity: "no-repository".into(),
                root,
                paths: vec![],
            };
            audit
                .assess(
                    &snapshot,
                    &[],
                    "no_repository",
                    &AuditDispatch(self.client.for_audit()),
                )
                .await?;
        }
        Ok(())
    }
    pub(super) async fn finalize_repository_audit(
        &self,
        mut summary: WorkflowV2ScriptSummary,
    ) -> WorkflowResult<WorkflowV2ScriptSummary> {
        let audit =
            self.client.audit.as_ref().ok_or_else(|| {
                WorkflowError::StateCorrupt("mandatory audit context missing".into())
            })?;
        let _boundary = audit.lock_write_boundary().await;
        let paths = audit
            .state()?
            .declared_paths
            .into_iter()
            .collect::<Vec<_>>();
        let snapshot = if let Some(root) = &self.runtime.target_repository_root {
            Snapshot::capture(std::path::Path::new(root), &paths, &self.v2_store)?
        } else {
            audit
                .state()?
                .snapshot
                .ok_or_else(|| WorkflowError::StateCorrupt("audit final snapshot missing".into()))?
        };
        let result = audit
            .assess(
                &snapshot,
                &paths,
                "final",
                &AuditDispatch(self.client.for_audit()),
            )
            .await
            .and_then(|_| audit.require_closed(&snapshot.identity))
            .and_then(|_| audit.seal_final(&snapshot.identity));
        if let Err(error) = result {
            if matches!(
                error,
                WorkflowError::ControlPaused(_) | WorkflowError::ControlCancelled(_)
            ) {
                return Err(error);
            }
            summary.status = WorkflowV2Status::Failed;
            summary.failed_call = Some("repository-audit-final".into());
            summary.failed_result_path = Some(
                self.v2_store
                    .root()
                    .join("repository-audit/state.json")
                    .display()
                    .to_string(),
            );
            summary.next_action = Some(error.to_string());
        }
        Ok(summary)
    }
}

/// This client is created only by the host. A script cannot select its tool policy.
pub(in super::super) struct AuditDispatch(pub(in super::super) LiveV2AgentClient);
#[async_trait::async_trait]
impl archon_workflow::WorkflowAgentDispatch for AuditDispatch {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn run_call(
        &self,
        task: &str,
        root: Option<String>,
        execution: &WorkflowV2CallExecution,
        adapter: &WorkflowV2AgentAdapter,
        store: Option<&WorkflowV2ResultStore>,
        _: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        let run_store = super::super::run_store_scope(store, root.as_deref(), None);
        let mut request =
            archon_workflow::v2::call_data::v2_agent_request(task, root, execution, None);
        request.role = "critic".into();
        let scope = store
            .map(|s| {
                archon_observability::transport::EvidenceScope::new(
                    s.root().join("transport.jsonl"),
                    &execution.call.id,
                )
            })
            .transpose()
            .map_err(|e| WorkflowError::StageFailed(e.to_string()))?;
        let (timeout, timeout_source) = match execution.call.options.extra.get("audit_timeout_secs")
        {
            Some(serde_json::Value::Null) => (None, "audit_timeout_secs"),
            Some(value) => (
                Some(value.as_u64().filter(|n| *n > 0).ok_or_else(|| {
                    WorkflowError::SpecInvalid("invalid host audit timeout".into())
                })?),
                "audit_timeout_secs",
            ),
            None => (self.0.timeout_secs(), self.0.timeout_source()),
        };
        let allowance = timeout
            .map(|seconds| format!("{seconds}s"))
            .unwrap_or_else(|| "unlimited".into());
        self.0
            .ui_sink
            .emit(WorkflowUiEvent::Text(format!(
                "Repository audit waiting: {} — allowance {}, snapshot {}\n",
                execution.call.id,
                allowance,
                execution
                    .input
                    .get("snapshot")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("not supplied")
            )))
            .await
            .map_err(|error| WorkflowError::NotificationDelivery(error.to_string()))?;
        let started = std::time::Instant::now();
        let client = self.0.with_timeout_secs(timeout, timeout_source);
        let call = archon_tools::workflow_read_guard::scope_run_store(
            run_store,
            Box::pin(adapter.run_with_repair(&client, &request)),
        );
        let call = async {
            match &scope {
                Some(s) => s.run(call).await,
                None => call.await,
            }
        };
        let result = if let Some(landing) = archon_workflow::repository_audit::landing::current() {
            let seconds = execution
                .call
                .options
                .extra
                .get("audit_path_timeout_secs")
                .and_then(serde_json::Value::as_u64)
                .map(|derived| derived.max(self.0.audit_min_progress_secs()));
            let tool = Arc::new(archon_tools::audit_landing::AuditLanding::new(
                Arc::new(LandingBridge(landing)),
                seconds,
            ));
            archon_tools::audit_landing::scope(tool, call).await
        } else {
            call.await
        };
        self.0
            .ui_sink
            .emit(WorkflowUiEvent::Text(format!(
                "Repository audit {}: {} after {:.1}s\n",
                execution.call.id,
                if result.is_ok() {
                    "assessment returned"
                } else {
                    "assessment failed"
                },
                started.elapsed().as_secs_f64()
            )))
            .await
            .map_err(|error| WorkflowError::NotificationDelivery(error.to_string()))?;
        if let Some(s) = scope {
            s.check()
                .map_err(|e| WorkflowError::StageFailed(e.to_string()))?;
        }
        result.map_err(|e| {
            WorkflowError::StageFailed(format!("repository audit assessment failed: {e}"))
        })
    }
}

#[cfg(test)]
mod declaration_tests {
    use super::*;
    use archon_workflow::repository_audit::budget::{AuditPolicy, Limit};

    type Fixture = (
        WorkflowStore,
        String,
        std::path::PathBuf,
        WorkflowV2ScriptRunner,
    );

    /// A sink that accepts every event and shows none of them.
    ///
    /// These tests exercise the audit's declaration handling, not delivery,
    /// and the fixture previously kept a TUI channel receiver alive only so
    /// the bounded sink had somewhere to send. Emitting through the port
    /// keeps this file free of `archon_tui`, which is the crate-boundary rule
    /// `workflow_crate_boundary_tests` holds every `workflow*.rs` file to.
    struct DiscardingSink;

    #[async_trait::async_trait]
    impl archon_workflow::ui_sink_port::WorkflowUiSink for DiscardingSink {
        async fn emit(
            &self,
            _event: archon_workflow::ui_sink_port::WorkflowUiEvent,
        ) -> archon_workflow::ui_sink_port::WorkflowUiResult {
            Ok(())
        }
    }

    /// An empty-tree repository ignoring `docs/*` (through `info/exclude`, so
    /// the sealed view stays empty and no assessor is needed), and a runner
    /// over it declaring `new.txt` and `docs/x.md` for one task.
    fn runner(temp: &std::path::Path) -> Fixture {
        let repo = temp.join("repo");
        std::fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "base",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(&repo)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        std::fs::write(repo.join(".git/info/exclude"), "/docs/*\n").unwrap();
        let project = temp.join("project");
        let store = WorkflowStore::project(&project);
        let run = store
            .create_run(archon_workflow::WorkflowSpec {
                schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
                name: "declarations".into(),
                task: "audit".into(),
                target_repository_root: Some(repo.display().to_string()),
                max_agents: 1,
                max_parallelism: 1,
                stages: vec![],
                permissions: Default::default(),
                learning_hooks: vec![],
            })
            .unwrap();
        struct NoProvider;
        #[async_trait::async_trait]
        impl archon_workflow::WorkflowLlmClient for NoProvider {
            async fn send_message(
                &self,
                _: Vec<serde_json::Value>,
                _: Vec<serde_json::Value>,
                _: Vec<serde_json::Value>,
                _: &str,
            ) -> WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
                panic!("empty repository does not need an assessor")
            }
        }
        let client = LiveV2AgentClient::new(
            Arc::new(NoProvider),
            Arc::new(DiscardingSink),
            vec![],
            run.id.clone(),
            Some(repo.display().to_string()),
            None,
        );
        let contract = |path: &str| archon_workflow::task_universe::WorkflowV2DeliverableContract {
            artifact_path: repo.join(path).display().to_string(),
            ..Default::default()
        };
        let universe = WorkflowV2TaskUniverse {
            tasks: vec![archon_workflow::task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "UNIT-1".into(),
                deliverable_contracts: vec![contract("new.txt"), contract("docs/x.md")],
                ..Default::default()
            }],
            schema_version: "workflow-v2-task-universe-v1".into(),
            source_roots: vec![],
        };
        let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
        let runner = WorkflowV2ScriptRunner::new(
            "audit".into(),
            WorkflowV2ScriptRuntime {
                target_repository_root: Some(repo.display().to_string()),
                ..Default::default()
            },
            WorkflowV2AgentAdapter::new(),
            client,
            v2,
            store.clone(),
            run.id.clone(),
            true,
            Some(universe),
            None,
        );
        (store, run.id, repo, runner)
    }

    #[tokio::test]
    async fn repository_audit_initialization_keeps_absolute_repository_declarations() {
        let temp = tempfile::tempdir().unwrap();
        let (_, _, _, mut runner) = runner(temp.path());
        runner.initialize_repository_audit().await.unwrap();
        let state = runner.client.audit.unwrap().state().unwrap();
        assert!(
            state.declared_paths.contains("new.txt"),
            "absolute in-repository declaration was silently omitted"
        );
        assert!(
            !state.declared_paths.contains("docs/x.md"),
            "a gitignored declaration is a project artifact, not a deliverable: {:?}",
            state.declared_paths
        );
        assert!(state.ledger.obligations.get("docs/x.md").is_none());
    }

    /// Issue-26, the live state: a run whose audit already carries the
    /// obligation stops looping on the next resume without operator surgery.
    #[tokio::test]
    async fn repository_audit_initialization_reclaims_an_ignored_obligation_from_existing_state() {
        use archon_workflow::repository_audit::{
            AuditContract, AuditReport,
            runtime::{AuditRuntime, Snapshot},
        };
        let temp = tempfile::tempdir().unwrap();
        let (store, run_id, repo, mut runner) = runner(temp.path());
        let audit = AuditRuntime::initialize(
            store.clone(),
            run_id.clone(),
            AuditPolicy {
                attempt_timeout_secs: Limit::Unlimited,
                total_time_secs: Limit::Unlimited,
                unexpected_change_refreshes: Limit::Finite(3),
            },
        )
        .unwrap();
        audit.update(|state| {
            state.declared_paths.extend(["docs/x.md".to_string(), "new.txt".into()]);
            state.snapshot = Some(Snapshot { identity:"stale".into(), root:repo.clone(), paths:vec![] });
            let report: AuditReport = serde_json::from_value(serde_json::json!({"schema_version":1,"snapshot":"stale","records":[
                {"declared_path":"docs/x.md","verdict":"absent","equivalents":[],"required_action":"deliver","reason":"never in the sealed tree"},
                {"declared_path":"new.txt","verdict":"absent","equivalents":[],"required_action":"deliver","reason":"not yet written"}]}))?;
            state.ledger.accept(AuditContract{schema_version:1,snapshot:"stale".into(),declared_paths:vec!["docs/x.md".into(),"new.txt".into()]},report)
        }).unwrap();
        runner.initialize_repository_audit().await.unwrap();
        let state = runner.client.audit.unwrap().state().unwrap();
        assert_eq!(
            state.declared_paths.iter().cloned().collect::<Vec<_>>(),
            vec!["new.txt".to_string()]
        );
        assert!(
            !state.ledger.obligations.contains_key("docs/x.md"),
            "{:?}",
            state.ledger.obligations
        );
        assert!(
            state.ledger.obligations.contains_key("new.txt"),
            "a real deliverable stays owed"
        );
        assert_eq!(
            state.ledger.history.len(),
            2,
            "the stale judgment stays, the initial assessment follows it"
        );
        assert!(state.ledger.ignored_paths.contains("docs/x.md"));
        let events = std::fs::read_to_string(store.events_path(&run_id)).unwrap();
        let dropped = events
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
            .filter(|r| r["detail"]["event"] == "repository_audit_ignored_paths_dropped")
            .collect::<Vec<_>>();
        assert_eq!(dropped.len(), 1, "{dropped:#?}");
        assert_eq!(
            dropped[0]["detail"]["paths"],
            serde_json::json!(["docs/x.md"])
        );
    }
}

#[path = "workflow_repository_audit_paths.rs"]
mod declaration_paths;

struct LandingBridge(Arc<archon_workflow::repository_audit::landing::AuditLanding>);
impl archon_tools::audit_landing::LandingHost for LandingBridge {
    fn land(&self, value: serde_json::Value) -> Result<String, String> {
        let record =
            serde_json::from_value(value).map_err(|e| format!("invalid AuditRecord: {e}"))?;
        self.0.land(record).map_err(|e| e.to_string())?;
        self.hint()
    }
    fn hint(&self) -> Result<String, String> {
        self.0.hint().map_err(|e| e.to_string())
    }
    fn complete(&self, value: &serde_json::Value) -> Result<(), String> {
        self.0
            .complete(value)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
