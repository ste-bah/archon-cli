//! Composition of the mandatory audit with provider-neutral workflow execution.
use super::*;
use archon_workflow::repository_audit::runtime::{AuditRuntime, Snapshot};
use archon_workflow::WorkflowResult;
use std::collections::BTreeSet;

impl WorkflowV2ScriptRunner {
    pub(super) async fn initialize_repository_audit(&mut self) -> WorkflowResult<()> {
        if self.client.audit.is_some() { return Ok(()); }
        let policy = self.client.audit_policy().unwrap_or_else(|| {
            use archon_workflow::repository_audit::budget::{AuditPolicy, Limit};
            AuditPolicy { attempt_timeout_secs: Limit::Finite(u64::from(self.runtime.generated_config.host_call_timeout_secs)),
                total_time_secs: Limit::Unlimited, unexpected_change_refreshes: Limit::Finite(3) }
        });
        let audit = AuditRuntime::initialize(self.workflow_store.clone(), self.run_id.clone(), policy)?;
        let paths = self.task_universe.as_ref().map(|universe| universe.tasks.iter()
            .flat_map(|t|t.deliverable_contracts.iter().map(|c|c.artifact_path.clone()))
            .collect::<BTreeSet<_>>()).unwrap_or_default();
        // Project artifacts and repository paths are distinct namespaces.
        // Repository declarations are selected using the existing root resolver.
        let repository_paths = paths.into_iter().filter(|p| !std::path::Path::new(p).is_absolute()
            && !p.starts_with("${") && !p.starts_with(".archon/")).collect::<Vec<_>>();
        audit.update(|s| { s.declared_paths.extend(repository_paths); Ok(()) })?;
        self.client = self.client.with_audit(audit.clone());
        if let Some(root) = &self.runtime.target_repository_root {
            let paths = audit.state()?.declared_paths.into_iter().collect::<Vec<_>>();
            let snapshot = Snapshot::capture(std::path::Path::new(root), &paths, &self.v2_store)?;
            let assessor = AuditDispatch(self.client.for_audit());
            audit.assess(&snapshot, &paths, "initial", &assessor).await?;
        } else {
            let root = self.v2_store.root().join("repository-audit/no-repository");
            std::fs::create_dir_all(&root).map_err(|e|WorkflowError::Io{path:root.clone(),source:e})?;
            let snapshot = Snapshot{identity:"no-repository".into(),root,paths:vec![]};
            audit.assess(&snapshot,&[],"no_repository",&AuditDispatch(self.client.for_audit())).await?;
        }
        Ok(())
    }
    pub(super) async fn finalize_repository_audit(&self, mut summary: WorkflowV2ScriptSummary) -> WorkflowResult<WorkflowV2ScriptSummary> {
        let audit=self.client.audit.as_ref().ok_or_else(||WorkflowError::StateCorrupt("mandatory audit context missing".into()))?;
        let paths=audit.state()?.declared_paths.into_iter().collect::<Vec<_>>();
        let snapshot=if let Some(root)=&self.runtime.target_repository_root {
            Snapshot::capture(std::path::Path::new(root),&paths,&self.v2_store)?
        }else{audit.state()?.snapshot.ok_or_else(||WorkflowError::StateCorrupt("audit final snapshot missing".into()))?};
        let result=audit.assess(&snapshot,&paths,"final",&AuditDispatch(self.client.for_audit())).await
            .and_then(|_|audit.require_closed(&snapshot.identity))
            .and_then(|_|audit.seal_final(&snapshot.identity));
        if let Err(error)=result {
            if matches!(error,WorkflowError::ControlPaused(_)|WorkflowError::ControlCancelled(_)){return Err(error);}
            summary.status=WorkflowV2Status::Failed;
            summary.failed_call=Some("repository-audit-final".into());
            summary.failed_result_path=Some(self.v2_store.root().join("repository-audit/state.json").display().to_string());
            summary.next_action=Some(error.to_string());
        }
        Ok(summary)
    }
}

/// This client is created only by the host. A script cannot select its tool policy.
pub(in super::super) struct AuditDispatch(pub(in super::super) LiveV2AgentClient);
#[async_trait::async_trait]
impl archon_workflow::WorkflowAgentDispatch for AuditDispatch {
    fn fanout_parallelism(&self,_:Option<usize>)->usize{1}
    async fn run_call(&self,task:&str,root:Option<String>,execution:&WorkflowV2CallExecution,adapter:&WorkflowV2AgentAdapter,
        store:Option<&WorkflowV2ResultStore>,_:Option<&WorkflowV2TaskUniverse>)->WorkflowResult<WorkflowV2Result>{
        let mut request=archon_workflow::v2::call_data::v2_agent_request(task,root,execution,None);
        request.role="critic".into();
        let scope=store.map(|s|archon_observability::transport::EvidenceScope::new(s.root().join("transport.jsonl"),&execution.call.id))
            .transpose().map_err(|e|WorkflowError::StageFailed(e.to_string()))?;
        let timeout = match execution.call.options.extra.get("audit_timeout_secs") {
            Some(serde_json::Value::Null) => None,
            Some(value) => Some(value.as_u64().filter(|n|*n>0).ok_or_else(||WorkflowError::SpecInvalid("invalid host audit timeout".into()))?),
            None => self.0.timeout_secs(),
        };
        let client = self.0.with_timeout_secs(timeout);
        let call=adapter.run_with_repair(&client,&request);
        let result=match &scope {Some(s)=>s.run(call).await,None=>call.await};
        if let Some(s)=scope{s.check().map_err(|e|WorkflowError::StageFailed(e.to_string()))?;}
        result.map_err(|e|WorkflowError::StageFailed(format!("repository audit assessment failed: {e}")))
    }
}

#[cfg(test)]
mod declaration_tests {
    use super::*;
    #[tokio::test]
    async fn repository_audit_initialization_keeps_absolute_repository_declarations() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        for args in [vec!["init","-q"],vec!["-c","user.name=fixture","-c","user.email=fixture@example.invalid","commit","--allow-empty","-qm","base"]] {
            assert!(std::process::Command::new("git").args(args).current_dir(&repo).status().unwrap().success());
        }
        let project = temp.path().join("project");
        let store = WorkflowStore::project(&project);
        let run = store.create_run(archon_workflow::WorkflowSpec {
            schema:archon_workflow::spec::WORKFLOW_SCHEMA.into(),name:"declarations".into(),task:"audit".into(),
            target_repository_root:Some(repo.display().to_string()),max_agents:1,max_parallelism:1,stages:vec![],permissions:Default::default(),learning_hooks:vec![],
        }).unwrap();
        struct NoProvider;
        #[async_trait::async_trait]
        impl archon_workflow::WorkflowLlmClient for NoProvider {
            async fn send_message(&self,_:Vec<serde_json::Value>,_:Vec<serde_json::Value>,_:Vec<serde_json::Value>,_:&str)->WorkflowResult<archon_workflow::WorkflowAgentOutcome>{panic!("empty repository does not need an assessor")}
        }
        let (ui,_rx)=crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
        let client=LiveV2AgentClient::new(Arc::new(NoProvider),ui,vec![],run.id.clone(),Some(repo.display().to_string()),None);
        let universe=WorkflowV2TaskUniverse{tasks:vec![archon_workflow::task_universe::WorkflowV2TaskUniverseTask{
            canonical_task_id:"UNIT-1".into(),deliverable_contracts:vec![archon_workflow::task_universe::WorkflowV2DeliverableContract{
                artifact_path:repo.join("new.txt").display().to_string(),..Default::default()
            }],..Default::default()}],..Default::default()};
        let v2=WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
        let mut runner=WorkflowV2ScriptRunner::new("audit".into(),WorkflowV2ScriptRuntime{target_repository_root:Some(repo.display().to_string()),..Default::default()},
            WorkflowV2AgentAdapter::new(),client,v2,store,run.id,true,Some(universe),None);
        runner.initialize_repository_audit().await.unwrap();
        assert!(runner.client.audit.unwrap().state().unwrap().declared_paths.contains("new.txt"),"absolute in-repository declaration was silently omitted");
    }
}
