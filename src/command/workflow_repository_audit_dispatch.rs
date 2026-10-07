//! Executor admission for the read-only repository assessor.
use super::*;
use archon_workflow::WorkflowResult;
/// This client is created only by the host. A script cannot select its tool policy.
pub(in super::super::super) struct AuditDispatch(pub(in super::super::super) LiveV2AgentClient);
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
        // The one ownership fence (Issue 291): the session's bound store
        // checks before every poll of the whole audit call, and never
        // recreates a run directory a successor removed.
        let work = self.run_owned(task, root, execution, adapter, store);
        match store {
            Some(v2) => {
                v2.session_workflow_store()?
                    .execute_owned(&v2.run_id(), work)
                    .await
            }
            None => work.await,
        }
    }
}

impl AuditDispatch {
    async fn run_owned(
        &self,
        task: &str,
        root: Option<String>,
        execution: &WorkflowV2CallExecution,
        adapter: &WorkflowV2AgentAdapter,
        store: Option<&WorkflowV2ResultStore>,
    ) -> WorkflowResult<WorkflowV2Result> {
        let run_store = super::super::super::run_store_scope(store, root.as_deref(), None);
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
        let mut client = self.0.with_timeout_secs(timeout, timeout_source);
        if let Some(v2) = store {
            client = client.with_owner_store(v2.session_workflow_store()?);
        }
        let call = archon_tools::workflow_read_guard::scope_run_store(
            run_store,
            Box::pin(adapter.run_with_repair(&client, &request)),
        );
        let call = async {
            match &scope {
                Some(s) => s.run(call).await,
                None => call.await,
            }
            .map_err(archon_workflow::WorkflowV2AgentError::into_workflow_error)
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
            if matches!(
                e,
                WorkflowError::ControlCancelled(_) | WorkflowError::ControlPaused(_)
            ) {
                return e;
            }
            WorkflowError::StageFailed(format!("repository audit assessment failed: {e}"))
        })
    }
}

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
