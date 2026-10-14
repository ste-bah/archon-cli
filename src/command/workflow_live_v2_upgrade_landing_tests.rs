//! A landing logic bump on resume, end to end through the script runner:
//! the superseded rounds of a body replay their recorded answers and only
//! the current body's request is judged again. Before, every landing the
//! script asked again ran under its new key and published its old
//! candidate over the body the later rounds had accepted.
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use super::*;

/// Real keys, `publishes` and logic stamps; a scripted gate: a candidate
/// whose stdin starts with `refused` gets a body finding, any other passes.
/// Every landing carries a receipt, as an observe-mode binary before this
/// fix published refused bodies too.
struct LandingHost {
    keys: FixedHostCommandExecutor,
    executed: Mutex<Vec<String>>,
    receipt_matches: AtomicBool,
    rejudge_findings: AtomicBool,
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
    fn landing_receipt_matches_disk(
        &self,
        record: &WorkflowV2CallRecord,
    ) -> archon_workflow::WorkflowResult<bool> {
        let outcome: archon_workflow::HostCommandResult =
            serde_json::from_value(record.result.data.clone())?;
        Ok(self.receipt_matches.load(Ordering::SeqCst) && outcome.publication_receipt.is_some())
    }
    async fn rejudge_landing(
        &self,
        record: &WorkflowV2CallRecord,
        generation: Option<u64>,
        _allow_restore: bool,
    ) -> archon_workflow::WorkflowResult<
        Option<crate::command::workflow_host_command_exec::LandingRejudgeOutcome>,
    > {
        let request = record.call.options.host_command.clone().unwrap();
        let mut outcome = self.execute(request, generation).await?;
        outcome.publication_receipt =
            serde_json::from_value(record.result.data["publicationReceipt"].clone()).ok();
        if self.rejudge_findings.load(Ordering::SeqCst) {
            let finding = archon_workflow::GatePolicyFinding {
                deterministic_defect: None,
                text: "current landing needs repair under this build".into(),
                subject: "TASK-X-001".into(),
                source_path: None,
                remediation_scope: archon_workflow::RemediationScope::Body,
            };
            if let Some(envelope) = outcome.gate_envelope.as_mut() {
                envelope.policy_findings = vec![finding];
                envelope.report = serde_json::json!("current landing finding");
            }
        }
        Ok(Some(
            crate::command::workflow_host_command_exec::LandingRejudgeOutcome {
                outcome,
                candidate: false,
            },
        ))
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

#[tokio::test]
async fn actionable_rejudge_observation_replays_without_replacing_publication_or_disk() {
    let run = Run::new();
    let task_path = run.context.task_root.join("TASK-X-001.md");
    std::fs::write(&task_path, b"current accepted bytes\n").unwrap();
    const ACCEPTED_LANDINGS: &str = r#"async function workflow(w) {
        const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
        await w.hostCommand("land-task-body", {stdin: "old accepted", target});
        await w.hostCommand("land-task-body", {stdin: "current accepted", target});
        return "seeded";
    }"#;
    const REPLAY_OLD: &str = r#"async function workflow(w) {
        const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
        return await w.hostCommand("land-task-body", {stdin: "old accepted", target});
    }"#;
    run.run(run.build(9), ACCEPTED_LANDINGS).await;
    let store = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let records_before = store.load_call_records().unwrap();
    let current_before = archon_workflow::v2::script::landing_lineage::LandingLineage::of(
        &records_before,
        "land-task-body",
        "TASK-X-001",
    )
    .current()
    .unwrap()
    .clone();
    let disk_before = std::fs::read(&task_path).unwrap();
    println!(
        "EXPECTED 447b current={} findings replay; disk unchanged",
        current_before.call.id
    );

    let first_host = run.build(13);
    first_host.rejudge_findings.store(true, Ordering::SeqCst);
    let first = run.run(first_host.clone(), REPLAY_OLD).await;
    assert_eq!(
        first["gateEnvelope"]["policy_findings"][0]["text"],
        "current landing needs repair under this build"
    );
    assert_eq!(
        first_host.executed.lock().unwrap().as_slice(),
        ["current accepted"]
    );
    assert_eq!(std::fs::read(&task_path).unwrap(), disk_before);
    let records_after_first = store.load_call_records().unwrap();
    let current_after_first = archon_workflow::v2::script::landing_lineage::LandingLineage::of(
        &records_after_first,
        "land-task-body",
        "TASK-X-001",
    )
    .current()
    .unwrap();
    assert_eq!(current_after_first.call.id, current_before.call.id);
    assert_eq!(current_after_first.result.data, current_before.result.data);
    assert_eq!(records_after_first.len(), records_before.len());
    let observation_dir = store.root().join("landing-rejudgements");
    let observation_path = std::fs::read_dir(&observation_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let observation: serde_json::Value =
        serde_json::from_slice(&std::fs::read(observation_path).unwrap()).unwrap();
    assert_eq!(observation["landing_call_id"], current_before.call.id);
    assert!(!observation["receipt_digest"].as_str().unwrap().is_empty());
    assert_eq!(observation["logic_version"], 13);
    assert!(observation["outcome"]["publicationReceipt"].is_null());

    let second_host = run.build(13);
    let second = run.run(second_host.clone(), REPLAY_OLD).await;
    assert_eq!(
        second["gateEnvelope"]["policy_findings"][0]["text"],
        "current landing needs repair under this build"
    );
    assert!(second_host.executed.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(&task_path).unwrap(), disk_before);
    assert_eq!(
        archon_workflow::v2::script::landing_lineage::LandingLineage::of(
            &store.load_call_records().unwrap(),
            "land-task-body",
            "TASK-X-001",
        )
        .current()
        .unwrap()
        .call
        .id,
        current_before.call.id
    );
    println!(
        "ACTUAL 447b findings replayed; extra judge calls=0; disk unchanged; current publication retained"
    );
}

#[tokio::test]
async fn rejudge_observation_preserves_duplicate_occurrence_identity() {
    let run = Run::new();
    let task_path = run.context.task_root.join("TASK-X-001.md");
    std::fs::write(&task_path, b"same accepted bytes\n").unwrap();
    const DUPLICATES: &str = r#"async function workflow(w) {
        const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
        for (let index = 0; index < 2; index++)
            await w.hostCommand("land-task-body", {stdin: "same body", target});
        return "seeded";
    }"#;
    const REPLAY_FIRST: &str = r#"async function workflow(w) {
        const target = {taskId: "TASK-X-001", fileName: "TASK-X-001.md"};
        return await w.hostCommand("land-task-body", {stdin: "same body", target});
    }"#;
    run.run(run.build(9), DUPLICATES).await;
    let store = WorkflowV2ResultStore::new(run.store.run_dir(&run.run_id).join("v2"));
    let before = store.load_call_records().unwrap();
    let matching = |record: &&WorkflowV2CallRecord, occurrence| {
        archon_workflow::v2::script::landing_lineage::occurrence_of(record) == occurrence
            && record
                .call
                .options
                .host_command
                .as_ref()
                .is_some_and(|request| request.stdin.as_deref() == Some("same body"))
    };
    let occurrence_one = before
        .iter()
        .find(|record| matching(record, 1))
        .unwrap()
        .clone();
    let occurrence_two = before
        .iter()
        .find(|record| matching(record, 2))
        .unwrap()
        .clone();
    println!(
        "EXPECTED 447b preserve {} and {} with no id collision",
        occurrence_one.call.id, occurrence_two.call.id
    );

    let first_host = run.build(13);
    first_host.rejudge_findings.store(true, Ordering::SeqCst);
    let first = run.run(first_host.clone(), REPLAY_FIRST).await;
    assert_eq!(
        first["gateEnvelope"]["policy_findings"][0]["text"],
        "current landing needs repair under this build"
    );
    let after = store.load_call_records().unwrap();
    assert!(
        after
            .iter()
            .any(|record| record.call.id == occurrence_one.call.id)
    );
    let persisted_two = after
        .iter()
        .find(|record| record.call.id == occurrence_two.call.id)
        .unwrap();
    assert_eq!(
        archon_workflow::v2::script::landing_lineage::occurrence_of(persisted_two),
        2
    );
    assert!(
        crate::command::workflow_host_command_occurrence::record_identity_matches(
            persisted_two,
            &run.build(9)
                .call_identity(persisted_two.call.options.host_command.as_ref().unwrap())
                .unwrap()
        )
    );
    let current_logic_identity = run
        .build(13)
        .call_identity(persisted_two.call.options.host_command.as_ref().unwrap())
        .unwrap();
    assert!(!after.iter().any(|record| {
        record.call.id == current_logic_identity
            && archon_workflow::v2::script::landing_lineage::occurrence_of(record) == 2
    }));
    assert_ne!(occurrence_one.call.id, occurrence_two.call.id);
    assert_eq!(std::fs::read(&task_path).unwrap(), b"same accepted bytes\n");

    let second_host = run.build(13);
    let second = run.run(second_host.clone(), REPLAY_FIRST).await;
    assert_eq!(
        second["gateEnvelope"]["policy_findings"][0]["text"],
        "current landing needs repair under this build"
    );
    assert!(second_host.executed.lock().unwrap().is_empty());
    println!(
        "ACTUAL 447b occurrences 1 and 2 intact; id collision=false; finding replayed without rejudge"
    );
}

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
            receipt_matches: AtomicBool::new(true),
            rejudge_findings: AtomicBool::new(false),
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
        let summary = self
            .run_summary_with_lease(host, script, lease)
            .await
            .unwrap();
        assert!(summary.script_error.is_none(), "{:?}", summary.script_error);
        serde_json::from_str(summary.script_result.as_deref().unwrap_or("null")).unwrap()
    }

    async fn run_summary_with_lease(
        &self,
        host: Arc<dyn WorkflowHostCommandExecutor>,
        script: &str,
        lease: Option<Arc<crate::command::workflow_executor_lease::ExecutionLease>>,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
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
        runner.run(script).await
    }
}

#[path = "workflow_live_v2_upgrade_landing_real_tests.rs"]
mod real_host_tests;
