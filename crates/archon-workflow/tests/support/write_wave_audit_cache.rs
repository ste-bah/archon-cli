use super::*;
use archon_workflow::repository_audit::{
    runtime::{AuditRuntime, Snapshot}, budget::{AuditPolicy, Limit}, AuditContract,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Audited {
    runtime: AuditRuntime,
    writer: Scripted,
    assessments: AtomicUsize,
    duplicate: bool,
    external_root: Option<PathBuf>,
}
#[async_trait::async_trait]
impl WorkflowAgentDispatch for Audited {
    fn repository_audit(&self) -> Option<AuditRuntime> { Some(self.runtime.clone()) }
    fn fanout_parallelism(&self, _: Option<usize>) -> usize { 1 }
    async fn run_call(&self, task: &str, root: Option<String>, execution: &WorkflowV2CallExecution,
        adapter: &WorkflowV2AgentAdapter, store: Option<&WorkflowV2ResultStore>,
        universe: Option<&task_universe::WorkflowV2TaskUniverse>) -> WorkflowResult<WorkflowV2Result>
    {
        if let Some(contract) = execution.call.options.extra.get("repository_audit_contract") {
            self.assessments.fetch_add(1, Ordering::SeqCst);
            let contract: AuditContract = serde_json::from_value(contract.clone())?;
            let root = PathBuf::from(root.unwrap());
            let records = contract.declared_paths.iter().map(|path| {
                if self.duplicate && path == "added.txt" {
                    return json!({"declared_path":path,"verdict":"exists_elsewhere","equivalents":["owned.txt"],
                        "required_action":"wire_or_migrate","reason":"existing behavior in sealed source"});
                }
                let exists = root.join(path).exists();
                json!({"declared_path":path,"verdict":if exists {"exists_as_declared"} else {"absent"},
                    "equivalents":[],"required_action":if exists {"none"} else {"deliver"},"reason":"inspected sealed fixture"})
            }).collect::<Vec<_>>();
            let mut result = WorkflowV2Result::accepted("assessed sealed source");
            result.data = json!({"repository_audit":{"schema_version":1,"snapshot":contract.snapshot,"records":records}});
            return Ok(result);
        }
        let result = self.writer.run_call(task, root, execution, adapter, store, universe).await;
        if execution.call.write_mode.is_some() && let Some(root) = &self.external_root {
            std::fs::write(root.join("outside-wave.custom"), "concurrent operator change").unwrap();
        }
        result
    }
}

#[tokio::test]
async fn repository_audit_rechecks_current_source_before_branch_cache_reuse() {
    let fixture = Fixture::new();
    let (first, _) = fixture.wave("cached-wave", Reply::Accepted).await;
    assert_eq!(first.status, WorkflowV2Status::Accepted);
    let runtime = AuditRuntime::initialize(fixture.store.clone(), fixture.run.clone(), AuditPolicy {
        attempt_timeout_secs: Limit::Unlimited, total_time_secs: Limit::Unlimited,
        unexpected_change_refreshes: Limit::Unlimited,
    }).unwrap();
    let dispatch = Audited { runtime, writer: Scripted { reply: Reply::Accepted,
        prompts: Mutex::new(vec![]), resumed: Mutex::new(false) }, assessments: AtomicUsize::new(0), duplicate: false, external_root: None };
    let paths = vec!["owned.txt".into(), "added.txt".into()];
    let snapshot = Snapshot::capture(&fixture.repo, &paths, &fixture.v2).unwrap();
    dispatch.runtime.assess(&snapshot, &paths, "initial", &dispatch).await.unwrap();
    std::fs::remove_file(fixture.repo.join("added.txt")).unwrap();
    let result = fixture.wave_with_dispatch("cached-wave", &dispatch).await;
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert!(fixture.repo.join("added.txt").exists(), "stale accepted branch bypassed current audit and delivery");
    assert!(!dispatch.writer.prompts.lock().unwrap().is_empty(), "stale cache prevented the required writer execution");
    dispatch.runtime.require_closed(&dispatch.runtime.state().unwrap().snapshot.unwrap().identity).unwrap();
}

#[tokio::test]
async fn repository_audit_serial_and_coordinated_cannot_bypass_preapply_gate() {
    for mode in [WorkflowV2WriteMode::Serial, WorkflowV2WriteMode::Coordinated] {
        let fixture = Fixture::new();
        let runtime = AuditRuntime::initialize(fixture.store.clone(), fixture.run.clone(), AuditPolicy {
            attempt_timeout_secs: Limit::Unlimited, total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Unlimited,
        }).unwrap();
        let dispatch = Audited { runtime, writer: Scripted { reply: Reply::Accepted,
            prompts: Mutex::new(vec![]), resumed: Mutex::new(false) },
            assessments: AtomicUsize::new(0), duplicate: true, external_root: None };
        let result = fixture.wave_with_mode("duplicate", &dispatch, mode).await;
        assert_ne!(result.status, WorkflowV2Status::Accepted, "{mode:?} bypassed audit");
        assert_eq!(git(&fixture.repo, &["rev-parse", "HEAD"]), fixture.base);
        assert!(!fixture.repo.join("added.txt").exists());
        assert!(dispatch.assessments.load(Ordering::SeqCst) > 0);
    }
}

#[tokio::test]
async fn repository_audit_postapply_counts_unexpected_changes_outside_applied_patch() {
    let fixture = Fixture::new();
    let runtime = AuditRuntime::initialize(fixture.store.clone(), fixture.run.clone(), AuditPolicy {
        attempt_timeout_secs: Limit::Unlimited, total_time_secs: Limit::Unlimited,
        unexpected_change_refreshes: Limit::Finite(1),
    }).unwrap();
    let dispatch = Audited { runtime, writer: Scripted { reply: Reply::Accepted,
        prompts: Mutex::new(vec![]), resumed: Mutex::new(false) },
        assessments: AtomicUsize::new(0), duplicate: false, external_root: Some(fixture.repo.clone()) };
    let result = fixture.wave_with_dispatch("concurrent-edit", &dispatch).await;
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert_eq!(dispatch.runtime.state().unwrap().budget.unexpected_refreshes, 1,
        "post-apply trigger hid a concurrent change outside the applied patch");
    assert_eq!(std::fs::read_to_string(fixture.repo.join("outside-wave.custom")).unwrap(), "concurrent operator change");
}

#[tokio::test]
async fn repository_audit_applied_disposition_can_resolve_through_separate_wiring_file() {
    struct Wiring(AuditRuntime);
    #[async_trait::async_trait]
    impl WorkflowAgentDispatch for Wiring {
        fn repository_audit(&self) -> Option<AuditRuntime> { Some(self.0.clone()) }
        fn fanout_parallelism(&self, _: Option<usize>) -> usize { 1 }
        async fn run_call(&self, _: &str, root: Option<String>, execution: &WorkflowV2CallExecution,
            _: &WorkflowV2AgentAdapter, _: Option<&WorkflowV2ResultStore>,
            _: Option<&task_universe::WorkflowV2TaskUniverse>) -> WorkflowResult<WorkflowV2Result> {
            let root = PathBuf::from(root.unwrap());
            if let Some(contract) = execution.call.options.extra.get("repository_audit_contract") {
                let contract: AuditContract = serde_json::from_value(contract.clone())?;
                let wired = std::fs::read_to_string(root.join("owned.txt")).unwrap() == "wired\n";
                let records = contract.declared_paths.iter().map(|path| json!({
                    "declared_path":path,"verdict":if path == "added.txt" && !wired {"unreachable"} else {"exists_as_declared"},
                    "equivalents":[],"required_action":if path == "added.txt" && !wired {"wire_or_migrate"} else {"none"},"reason":"inspected wiring"
                })).collect::<Vec<_>>();
                let mut result = WorkflowV2Result::accepted("inspected wiring");
                result.data = json!({"repository_audit":{"schema_version":1,"snapshot":contract.snapshot,"records":records}});
                return Ok(result);
            }
            std::fs::write(root.join("owned.txt"), "wired\n").unwrap();
            let mut result = WorkflowV2Result::accepted("wired existing deliverable");
            result.files_changed.push(WorkflowV2FileRecord::new("owned.txt"));
            result.evidence.push(WorkflowV2Evidence::new(WorkflowV2EvidenceKind::Implementation,"wired the entry point"));
            result.commands_run.push(WorkflowV2CommandRecord { kind:WorkflowV2CommandKind::Test,
                command:"test -s owned.txt".into(),status:WorkflowV2CommandStatus::Succeeded,exit_code:Some(0),output_summary:"present".into() });
            let snapshot = self.0.state()?.snapshot.unwrap().identity;
            result.data = json!({"audit_dispositions":[{"declared_path":"added.txt","snapshot":snapshot,
                "explanation":"wired through the entry point","evidence_paths":["owned.txt"]}]});
            Ok(result)
        }
    }
    let fixture = Fixture::new();
    std::fs::write(fixture.repo.join("added.txt"), "existing implementation\n").unwrap();
    git(&fixture.repo, &["add", "added.txt"]);
    git(&fixture.repo, &["commit", "-qm", "existing disconnected deliverable"]);
    let audit = AuditRuntime::initialize(fixture.store.clone(),fixture.run.clone(),AuditPolicy{
        attempt_timeout_secs:Limit::Unlimited,total_time_secs:Limit::Unlimited,unexpected_change_refreshes:Limit::Unlimited}).unwrap();
    let dispatch = Wiring(audit);
    let result = fixture.wave_with_dispatch("wire-existing", &dispatch).await;
    assert_eq!(result.status,WorkflowV2Status::Accepted,"{result:#?}");
    let state = dispatch.0.state().unwrap();
    assert!(dispatch.0.require_closed(&state.snapshot.unwrap().identity).is_ok(),
        "applied wiring evidence was not credited to its semantic obligation");
    assert_eq!(state.ledger.obligations["added.txt"].proposed_explanation.as_deref(),Some("wired through the entry point"));
}
