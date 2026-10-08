//! Issue-367: the body gate's shape refusal, staged by the real refusal code,
//! reaches the author through the host executor as a retryable refusal with
//! its one `Body` finding, never as an integrity failure or a publication.
use std::sync::Arc;

use archon_workflow::{HostCommandRequest, RemediationScope, WorkflowResult};

use super::candidate::{normalize_task_candidate, stage_shape_refusal};
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

/// The child's candidate-shape step, run in process on the resolved argv.
struct ShapeRefusingChild;

fn arg<'a>(request: &'a ResolvedHostCommand, flag: &str) -> &'a str {
    request
        .args
        .windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].as_str())
        .unwrap_or_else(|| panic!("{flag} in {:?}", request.args))
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for ShapeRefusingChild {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        _control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        let candidate = request.stdin.clone().expect("candidate stdin");
        let reason = normalize_task_candidate(candidate).expect_err("the shape is refused");
        let manifest = stage_shape_refusal(
            std::path::Path::new(arg(&request, "--staging-root")),
            std::path::Path::new(arg(&request, "--gate-envelope")),
            arg(&request, "--call-id"),
            std::path::Path::new(arg(&request, "--task-file")),
            reason,
        )
        .unwrap();
        let stdout = serde_json::to_vec(&manifest).unwrap();
        Ok(SupervisedProcessOutput {
            exit_code: Some(0),
            timed_out: false,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: 0,
            stdout,
            stderr: Vec::new(),
        })
    }
}

#[tokio::test]
async fn the_live_shape_refusal_reaches_the_author_as_a_retryable_body_finding() {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    context.gate_mode = archon_core::config::GateMode::Enforce;
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"live-before").unwrap();
    seed_frozen_chain(&context, &task_file);
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "body-shape-refusal".into(),
            task: "refuse".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        run_root,
        Arc::new(ShapeRefusingChild),
    );
    let chat = "All facts verified. Authoring the repaired TASK body now.";
    let answer = format!(
        "{chat}\n\n```markdown\n```yaml\ntask_id: TASK-X-010\ntitle: Candidate\ncomplexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n## Plan\n\nText.\n```\n"
    );
    let request = HostCommandRequest::new("land-task-body", Some(answer)).unwrap();

    let result = executor
        .execute(request, Some(run.generation))
        .await
        .expect("a refusal is an outcome, not an error");

    assert!(result.publication_receipt.is_none(), "nothing lands");
    assert_eq!(
        result.postcondition.as_ref().map(|p| p.summary.as_str()),
        Some("candidate refused before staging")
    );
    let envelope = result.gate_envelope.expect("the refusal's envelope");
    assert!(envelope.operational_error.is_none());
    assert_eq!(envelope.policy_findings.len(), 1, "{envelope:?}");
    let finding = &envelope.policy_findings[0];
    assert_eq!(finding.remediation_scope, RemediationScope::Body);
    // Measured by the script as a refused attempt at the shape stage.
    let defect = finding
        .deterministic_defect
        .as_ref()
        .expect("host identity");
    assert_eq!(defect.code, "invalid_candidate_shape");
    assert_eq!(serde_json::to_value(defect).unwrap()["stage"], "shape");
    assert!(
        finding.text.starts_with(&format!(
            "the answer has text before the task file (first line: \"{chat}\")"
        )),
        "{}",
        finding.text
    );
    assert_eq!(std::fs::read(&task_file).unwrap(), b"live-before");
}
