//! A landing logic bump on resume, end to end through the script runner:
//! the superseded rounds of a body replay their recorded answers and only
//! the current body's request is judged again. Before, every landing the
//! script asked again ran under its new key and published its old
//! candidate over the body the later rounds had accepted.
use std::sync::Mutex;

use super::*;

/// Real keys, `publishes` and logic stamps; a scripted gate: a candidate
/// whose stdin starts with `refused` gets a body finding, any other passes.
/// Every landing carries a receipt, as an observe-mode binary before this
/// fix published refused bodies too.
struct LandingHost {
    keys: FixedHostCommandExecutor,
    executed: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowHostCommandExecutor for LandingHost {
    /// As the real key does: the command, its logic version above the
    /// baseline, and the exact stdin (the real one binds the frozen subject,
    /// which this fixture has no pin for).
    fn call_identity(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<String> {
        let version = self.keys.logic_version(request)?.unwrap_or_default();
        let stdin = request.stdin.as_deref().unwrap_or_default();
        Ok(format!(
            "{}-v{version}-{}",
            request.command_id,
            &archon_workflow::task_set_contract::content_digest(stdin.as_bytes())[..12]
        ))
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
    fn publishes(
        &self,
        request: &archon_workflow::HostCommandRequest,
    ) -> archon_workflow::WorkflowResult<bool> {
        self.keys.publishes(request)
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
        _generation: Option<u64>,
    ) -> archon_workflow::WorkflowResult<archon_workflow::HostCommandResult> {
        let stdin = request.stdin.clone().unwrap_or_default();
        self.executed.lock().unwrap().push(stdin.clone());
        let findings = if stdin.starts_with("refused") {
            vec![archon_workflow::GatePolicyFinding {
                deterministic_defect: None,
                text: format!("gap in {stdin}"),
                subject: "TASK-X-001".into(),
                source_path: None,
                remediation_scope: archon_workflow::RemediationScope::Body,
            }]
        } else {
            Vec::new()
        };
        Ok(archon_workflow::HostCommandResult {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            timed_out: false,
            interrupted: false,
            stdout_truncated: false,
            stderr_truncated: false,
            gate_envelope: Some(archon_workflow::GateEnvelopeV1 {
                schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
                report: serde_json::json!(stdin),
                policy_findings: findings,
                operational_error: None,
            }),
            publication_receipt: Some(archon_workflow::PublicationReceiptV1 {
                schema_version: archon_workflow::PUBLICATION_RECEIPT_SCHEMA_VERSION,
                call_id: stdin.clone(),
                command_id: request.command_id,
                entries: Vec::new(),
                committed_at: "2026-10-13T00:00:00Z".into(),
            }),
            subjects: Vec::new(),
            postcondition: Some(archon_workflow::CommandPostconditionEvaluation {
                satisfied: true,
                summary: "fixture postcondition".into(),
            }),
        })
    }
}

/// One body's author loop: each candidate is landed until one passes.
const SCRIPT: &str = r#"async function workflow(w) {
    const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
    for (const body of ["refused round 1", "refused round 2", "round 3"]) {
        const outcome = await w.hostCommand("land-task-body", {stdin: body, target});
        const sentBack = (outcome.gateEnvelope?.policy_findings || [])
            .some((finding) => finding.remediation_scope === "body");
        if (!sentBack) return outcome.gateEnvelope.report;
    }
    throw new Error("no round passed");
}"#;

/// The damaged-run shape: an old round asked alone, as a buggy resume did.
const ROUND_ONE: &str = r#"async function workflow(w) {
    const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
    return await w.hostCommand("land-task-body", {stdin: "refused round 1", target});
}"#;

struct Run {
    _temp: tempfile::TempDir,
    spec: WorkflowSpec,
    store: WorkflowStore,
    run_id: String,
    context: crate::command::workflow_host_command_catalog::HostCommandResolutionContext,
}

impl Run {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let spec = test_spec();
        let context = crate::command::workflow_host_command_exec_tests::context(temp.path());
        let store = WorkflowStore::project(&context.project_root);
        let run = store.create_run(spec.clone()).unwrap();
        Self {
            _temp: temp,
            spec,
            store,
            run_id: run.id,
            context,
        }
    }

    /// A build whose land-task-body logic is `version`.
    fn build(&self, version: u32) -> Arc<LandingHost> {
        Arc::new(LandingHost {
            keys: FixedHostCommandExecutor::new(
                launch_catalog(),
                self.context.clone(),
                self.store.run_dir(&self.run_id),
            )
            .with_logic_version("land-task-body", Some(version)),
            executed: Mutex::new(Vec::new()),
        })
    }

    async fn run(
        &self,
        host: Arc<dyn WorkflowHostCommandExecutor>,
        script: &str,
    ) -> serde_json::Value {
        self.run_with_lease(host, script, None).await
    }

    async fn run_with_lease(
        &self,
        host: Arc<dyn WorkflowHostCommandExecutor>,
        script: &str,
        lease: Option<Arc<crate::command::workflow_executor_lease::ExecutionLease>>,
    ) -> serde_json::Value {
        let (sink, _rx) = default_workflow_ui_sink();
        let runner = WorkflowV2ScriptRunner::new(
            "landing upgrade".into(),
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
            // Each resume is a new session of the run's store.
            WorkflowV2ResultStore::new(self.store.run_dir(&self.run_id).join("v2")),
            self.store.clone(),
            self.run_id.clone(),
            true,
            None,
            None,
        )
        .with_raw_outcomes(true)
        .with_host_command_executor(host);
        let runner = if let Some(lease) = lease {
            let generation = self.store.load_state(&self.run_id).unwrap().generation;
            runner.with_executor_lease(lease, generation)
        } else {
            runner
        };
        let summary = runner.run(script).await.unwrap();
        let summary = summary;
        assert!(summary.script_error.is_none(), "{:?}", summary.script_error);
        serde_json::from_str(summary.script_result.as_deref().unwrap_or("null")).unwrap()
    }
}

#[path = "workflow_live_v2_upgrade_landing_real_tests.rs"]
mod real_host_tests;
