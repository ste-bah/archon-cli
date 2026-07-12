use std::collections::VecDeque;
use std::sync::Mutex;

use archon_workflow::{
    WorkflowV2AgentAdapter, WorkflowV2AgentClient, WorkflowV2AgentError, WorkflowV2AgentRequest,
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
    WorkflowV2WriteMode,
};

fn request(role: &str, write_mode: Option<WorkflowV2WriteMode>) -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "call-1".to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode,
            options: Default::default(),
        },
        role: role.to_string(),
        task: "Inspect or implement the requested work".to_string(),
        constraints: vec!["return typed evidence".to_string()],
        input: serde_json::json!({ "task_id": "T001" }),
        repository_root: Some("/repo".to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    }
}

fn accepted_json() -> String {
    serde_json::to_string(&accepted_result()).expect("serialize")
}

fn accepted_result() -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("implemented concrete change");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "changed src/lib.rs",
    ));
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/lib.rs"));
    result
}

fn noop_json() -> String {
    let mut result = WorkflowV2Result::noop("already implemented");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "source already satisfies task",
    ));
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "T001".to_string(),
        status: WorkflowV2TaskCoverageStatus::Noop,
        summary: "T001 already present".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Inspection,
            "verified existing implementation",
        )],
    });
    serde_json::to_string(&result).expect("serialize")
}

struct FakeAgentClient {
    responses: Mutex<VecDeque<Result<String, WorkflowV2AgentError>>>,
    prompts: Mutex<Vec<String>>,
}

impl FakeAgentClient {
    fn new(responses: Vec<Result<String, WorkflowV2AgentError>>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            prompts: Mutex::new(Vec::new()),
        }
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts.lock().expect("prompts").clone()
    }
}

#[async_trait::async_trait]
impl WorkflowV2AgentClient for FakeAgentClient {
    async fn run_agent(&self, prompt: String) -> Result<String, WorkflowV2AgentError> {
        self.prompts.lock().expect("prompts").push(prompt);
        self.responses
            .lock()
            .expect("responses")
            .pop_front()
            .expect("response")
    }
}

#[test]
fn prompt_requires_typed_result_envelope_without_provider_words() {
    let adapter = WorkflowV2AgentAdapter::new();
    let prompt = adapter.build_prompt(&request("researcher", None));
    let lower = prompt.to_ascii_lowercase();

    assert!(prompt.contains("Required JSON Result Envelope"));
    assert!(prompt.contains("\"task_coverage\""));
    assert!(prompt.contains("Return exactly one JSON object"));
    assert!(!lower.contains("claude"));
    assert!(!lower.contains("openai"));
    assert!(!lower.contains("sonnet"));
    assert!(!lower.contains("gpt"));
}

#[test]
fn implementation_prompt_requires_edits_or_typed_noop_proof() {
    let adapter = WorkflowV2AgentAdapter::new();
    let prompt = adapter.build_prompt(&request("coder", Some(WorkflowV2WriteMode::Coordinated)));

    assert!(prompt.contains("files_changed must list each changed path"));
    assert!(prompt.contains("status must be noop and task_coverage must include typed evidence"));
    assert!(prompt.contains("Status accepted with no files_changed is invalid"));
}

#[tokio::test]
async fn malformed_output_gets_one_schema_repair_retry() {
    let adapter = WorkflowV2AgentAdapter::new();
    let client = FakeAgentClient::new(vec![Ok("markdown only".to_string()), Ok(accepted_json())]);

    let result = adapter
        .run_with_repair(
            &client,
            &request("coder", Some(WorkflowV2WriteMode::Serial)),
        )
        .await
        .expect("repair should succeed");

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(client.prompts().len(), 2);
    assert!(client.prompts()[1].contains("previous workflow V2 agent response"));
    assert!(client.prompts()[1].contains("Required JSON Result Envelope"));
    assert!(client.prompts()[1].contains("\"task_coverage\""));
    assert!(client.prompts()[1].contains("Declared target_files"));
    assert!(client.prompts()[1].trim_end().ends_with("Status: noop."));
}

#[tokio::test]
async fn repeated_malformed_output_fails_with_exact_error() {
    let adapter = WorkflowV2AgentAdapter::new();
    let client = FakeAgentClient::new(vec![
        Ok("markdown only".to_string()),
        Ok("still markdown".to_string()),
    ]);

    let err = adapter
        .run_with_repair(&client, &request("researcher", None))
        .await
        .expect_err("repair should fail");

    assert!(matches!(err, WorkflowV2AgentError::RepairExhausted { .. }));
    assert_eq!(client.prompts().len(), 2);
}

#[test]
fn restored_context_summary_is_rejected() {
    let adapter = WorkflowV2AgentAdapter::new();
    let err = adapter
        .parse_agent_output(
            &request("researcher", None),
            "Restored context summary: previous run did something",
        )
        .expect_err("restored context must fail");

    assert_eq!(err, WorkflowV2AgentError::RestoredContextSummary);
}

#[test]
fn implementation_accepted_without_changed_files_is_rejected() {
    let adapter = WorkflowV2AgentAdapter::new();
    let mut result = WorkflowV2Result::accepted("looks done");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "no files changed",
    ));
    let raw = serde_json::to_string(&result).expect("serialize");

    let err = adapter
        .parse_agent_output(&request("coder", Some(WorkflowV2WriteMode::Serial)), &raw)
        .expect_err("accepted without edits must fail");

    assert_eq!(
        err,
        WorkflowV2AgentError::ImplementationAcceptedWithoutChanges
    );
}

#[test]
fn implementation_changed_files_must_stay_inside_declared_targets() {
    let adapter = WorkflowV2AgentAdapter::new();
    let mut result = accepted_result();
    result.files_changed = vec![WorkflowV2FileRecord::new("src/main.rs")];
    let raw = serde_json::to_string(&result).expect("serialize");

    let err = adapter
        .parse_agent_output(&request("coder", Some(WorkflowV2WriteMode::Serial)), &raw)
        .expect_err("changed file outside ownership must fail");

    assert!(matches!(
        err,
        WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(message)
            if message.contains("src/main.rs")
    ));
}

#[test]
fn implementation_changed_files_require_declared_targets() {
    let adapter = WorkflowV2AgentAdapter::new();
    let mut request = request("coder", Some(WorkflowV2WriteMode::Serial));
    request.target_files.clear();
    let raw = accepted_json();

    let err = adapter
        .parse_agent_output(&request, &raw)
        .expect_err("changed file without ownership must fail");

    assert!(matches!(
        err,
        WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(message)
            if message.contains("declares no target ownership")
    ));
}

#[test]
fn read_only_result_with_changed_files_is_rejected() {
    let adapter = WorkflowV2AgentAdapter::new();
    let raw = accepted_json();

    let err = adapter
        .parse_agent_output(&request("researcher", None), &raw)
        .expect_err("read-only agent cannot claim changed files");

    assert_eq!(err, WorkflowV2AgentError::ReadOnlyChangedFiles);
}

#[test]
fn read_only_result_accepts_common_schema_aliases() {
    let adapter = WorkflowV2AgentAdapter::new();
    let raw = r#"{
        "status": "completed",
        "summary": "Read-only audit completed.",
        "evidence": [
            { "kind": "inspect", "summary": "Inspected source and task files." }
        ],
        "commands_run": [
            {
                "kind": "inspection",
                "command": "rg TASK src",
                "status": "succeeded",
                "exit_code": 0,
                "output_summary": "Found implementation evidence."
            }
        ],
        "task_coverage": [
            {
                "task_id": "T001",
                "status": "completed",
                "summary": "Task already implemented.",
                "evidence": [
                    { "kind": "inspect", "summary": "Existing implementation inspected." }
                ]
            }
        ]
    }"#;

    let result = adapter
        .parse_agent_output(&request("researcher", None), raw)
        .expect("aliases should parse");

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.evidence[0].kind, WorkflowV2EvidenceKind::Inspection);
    assert_eq!(result.commands_run[0].kind, WorkflowV2CommandKind::Inspect);
    assert_eq!(
        result.task_coverage[0].status,
        WorkflowV2TaskCoverageStatus::Accepted
    );
}

#[test]
fn read_only_result_accepts_task_file_evidence_alias() {
    let adapter = WorkflowV2AgentAdapter::new();
    let raw = r#"{
        "status": "accepted",
        "summary": "Artifact inventory used task-file evidence.",
        "evidence": [
            {
                "kind": "task_file",
                "summary": "Read TASK-TDL files to identify required artifacts."
            }
        ],
        "files_read": [
            {
                "path": "/Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001/TASK-TDL-080-coverage-matrix-command.md"
            }
        ]
    }"#;

    let result = adapter
        .parse_agent_output(&request("reducer", None), raw)
        .expect("task_file evidence should parse at the live agent boundary");

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.evidence[0].kind, WorkflowV2EvidenceKind::Inspection);
}

#[test]
fn read_only_test_evidence_without_successful_test_command_is_inspection() {
    let adapter = WorkflowV2AgentAdapter::new();
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "Read-only audit inspected tests without running them.".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Test,
            "Inspected existing test coverage.",
        )],
        commands_run: vec![WorkflowV2CommandRecord {
            kind: WorkflowV2CommandKind::Test,
            command: "cargo test focused".to_string(),
            status: WorkflowV2CommandStatus::Skipped,
            exit_code: None,
            output_summary: "Read-only audit did not execute tests.".to_string(),
        }],
        task_coverage: vec![WorkflowV2TaskCoverage {
            task_id: "T001".to_string(),
            status: WorkflowV2TaskCoverageStatus::Partial,
            summary: "Implementation evidence exists, tests not run.".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Test,
                "Inspected test file names.",
            )],
        }],
        ..WorkflowV2Result::default()
    };
    let raw = serde_json::to_string(&result).expect("serialize");

    result = adapter
        .parse_agent_output(&request("researcher", None), &raw)
        .expect("read-only test inspection should not claim a test run");

    assert_eq!(result.evidence[0].kind, WorkflowV2EvidenceKind::Inspection);
    assert_eq!(
        result.task_coverage[0].evidence[0].kind,
        WorkflowV2EvidenceKind::Inspection
    );
}

#[test]
fn implementation_test_evidence_still_requires_successful_test_command() {
    let adapter = WorkflowV2AgentAdapter::new();
    let mut result = accepted_result();
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Test,
        "Tests were inspected but not run.",
    ));
    result.commands_run.push(WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test focused".to_string(),
        status: WorkflowV2CommandStatus::Skipped,
        exit_code: None,
        output_summary: "Not executed.".to_string(),
    });
    let raw = serde_json::to_string(&result).expect("serialize");

    let err = adapter
        .parse_agent_output(&request("coder", Some(WorkflowV2WriteMode::Serial)), &raw)
        .expect_err("implementation test evidence still requires command proof");

    assert!(
        matches!(err, WorkflowV2AgentError::InvalidResult(message) if message.contains("test evidence"))
    );
}

#[test]
fn typed_noop_proof_is_accepted_for_implementation() {
    let adapter = WorkflowV2AgentAdapter::new();
    let result = adapter
        .parse_agent_output(
            &request("coder", Some(WorkflowV2WriteMode::Serial)),
            &noop_json(),
        )
        .expect("typed noop proof");

    assert_eq!(result.status, WorkflowV2Status::Noop);
}
