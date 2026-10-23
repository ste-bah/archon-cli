//! A staged acceptance freeze prepared against a contract that is no longer
//! the published one, through the real executor: nothing is published, and
//! the outcome carries the contract published now, read by the host, for
//! the author loop to rebuild its candidate from.

use std::sync::Arc;

use archon_workflow::{HostCommandRequest, RunStatus, StageStatus, WorkflowStore};

use crate::command::workflow_host_command_catalog::{
    ResolvedHostCommand, fixed_decomposition_catalog,
};
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, HostCommandProcessAdapter, WorkflowHostCommandExecutor,
};
use crate::command::workflow_host_command_exec_tests::{context, seed_frozen_chain};
use crate::command::workflow_host_command_supervisor::{
    HostCommandControl, SupervisedProcessOutput,
};

/// A freeze child that stages every declared output and says it prepared
/// them against `prepared_against`, a contract digest.
struct FreezePreparedAgainst {
    prepared_against: String,
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for FreezePreparedAgainst {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        _control: HostCommandControl,
    ) -> archon_workflow::WorkflowResult<SupervisedProcessOutput> {
        let envelope = serde_json::to_vec_pretty(&archon_workflow::GateEnvelopeV1 {
            schema_version: archon_workflow::GATE_ENVELOPE_SCHEMA_VERSION,
            report: serde_json::json!("judged"),
            policy_findings: Vec::new(),
            operational_error: None,
        })
        .unwrap();
        let mut entries = Vec::new();
        for path in &request.declared_write_set {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let bytes = if name == "gate-envelope.json" {
                envelope.clone()
            } else {
                format!("staged {name}").into_bytes()
            };
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, &bytes).unwrap();
            entries.push(archon_workflow::PreparedPublicationEntry {
                relative_path: name,
                byte_len: bytes.len() as u64,
                blake3: archon_workflow::task_set_contract::content_digest(&bytes),
            });
        }
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let call_id = request
            .args
            .windows(2)
            .find(|pair| pair[0] == "--call-id")
            .map(|pair| pair[1].clone())
            .unwrap();
        let stdout = serde_json::to_vec(&archon_workflow::PreparedPublicationV1 {
            schema_version: archon_workflow::PREPARED_PUBLICATION_SCHEMA_VERSION,
            call_id,
            command_id: request.command_id.clone(),
            entries,
            expected_acceptance_contract: Some(archon_workflow::v2::ExpectedAcceptanceContract {
                exists: true,
                digest: Some(self.prepared_against.clone()),
            }),
        })
        .unwrap();
        let bytes = stdout.len() as u64;
        Ok(SupervisedProcessOutput {
            exit_code: Some(0),
            timed_out: false,
            stdout_bytes: bytes,
            stderr_bytes: 0,
            stdout_retained_bytes: bytes,
            stderr_retained_bytes: 0,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout,
            stderr: Vec::new(),
            stdout_path: None,
            stderr_path: None,
        })
    }
}

#[tokio::test]
async fn a_stale_freeze_outcome_carries_the_contract_published_now() {
    use archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    // A PRD with obligations: the acceptance freeze binds the launch PRD.
    std::fs::write(
        &context.prd_path,
        "# PRD X\n\n## Requirements\n| ID | Requirement |\n|---|---|\n| REQ-X-001 | Initial |\n\n## Acceptance\n| ID | Criterion |\n|---|---|\n| AC-X-001 | Initial |\n",
    )
    .unwrap();
    context.prd_digest = crate::command::workflow_task_set::validate_prd_input(&context.prd_path)
        .unwrap()
        .1;
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"body").unwrap();
    seed_frozen_chain(&context, &task_file);
    // The contract a later per-check repair published: what the loop must
    // rebuild from, not what the freeze was prepared against.
    let published = serde_json::json!({"acceptance": [{"id": "AC-X-001", "version": "published"}],
                                       "supplementary": []});
    let contract = context.task_root.join(ACCEPTANCE_CONTRACT_FILE);
    std::fs::write(&contract, serde_json::to_vec(&published).unwrap()).unwrap();
    let store = WorkflowStore::project(&context.project_root);
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "stale-freeze".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    run.status = RunStatus::Running;
    let mut stage = archon_workflow::run::StageState::pending("host-call");
    stage.status = StageStatus::Running;
    run.stages.insert("host-call".into(), stage);
    store.save_state(&run).unwrap();
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context.clone(),
        run_root,
        Arc::new(FreezePreparedAgainst {
            prepared_against: archon_workflow::task_set_contract::content_digest(b"{}"),
        }),
    );
    let request = HostCommandRequest::new("freeze-acceptance", Some("candidate".into())).unwrap();
    let outcome = executor
        .execute(request, Some(run.generation))
        .await
        .unwrap();
    assert!(outcome.publication_receipt.is_none(), "{outcome:?}");
    assert_eq!(
        std::fs::read(&contract).unwrap(),
        serde_json::to_vec(&published).unwrap(),
        "nothing was published over the current contract"
    );
    let envelope = outcome.gate_envelope.expect("a stale envelope");
    assert_eq!(
        envelope.report["current_acceptance_contract"], published,
        "the host read the contract published now"
    );
    assert_eq!(envelope.report["gate_report"], "judged");
    assert!(envelope.policy_findings.iter().any(|finding| {
        finding
            .deterministic_defect
            .as_ref()
            .is_some_and(|defect| defect.code == "stale_acceptance_contract")
    }));
}
