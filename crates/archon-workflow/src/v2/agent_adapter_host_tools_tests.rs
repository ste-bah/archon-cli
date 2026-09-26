// Issue-116: the required-tool proof credits what the host saw the session
// run, and one repair turn names every contract violation of a result.
use super::*;
use crate::v2::host_tool_log::{HostToolLog, scope};
use crate::{
    WorkflowV2Artifact, WorkflowV2CommandKind, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2WriteMode,
};
use std::collections::VecDeque;
use std::sync::Mutex;

const TOOLS: [&str; 5] = [
    "mcp__tradingview__tv_health_check",
    "mcp__tradingview__chart_get_state",
    "mcp__tradingview__tv_ui_state",
    "mcp__tradingview__data_get_ohlcv",
    "mcp__tradingview__tv_discover",
];

struct Fixture {
    _temp: tempfile::TempDir,
    request: WorkflowV2AgentRequest,
    sidecar: std::path::PathBuf,
    artifact: String,
    project_root: std::path::PathBuf,
}

/// The live shape: a write branch declaring the five MCP tools, with the
/// project artifact root of a run.
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let project_root = temp.path().join("project");
    let repo_root = temp.path().join("repo");
    let run_id = "wf-host-tools";
    let v2_root = project_root
        .join(".archon/workflows")
        .join(run_id)
        .join("v2");
    std::fs::create_dir_all(&v2_root).unwrap();
    std::fs::create_dir_all(repo_root.join("src")).unwrap();
    let request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "review-remediate-x-1-69-0".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "remediate".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({"item": {"required_tools": TOOLS}}),
        repository_root: Some(repo_root.display().to_string()),
        project_artifacts: crate::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    };
    Fixture {
        sidecar: v2_root.join("read-sets/branch.jsonl"),
        artifact: format!(".archon/workflows/{run_id}/artifacts/x-1-69-0-remediation.json"),
        project_root,
        request,
        _temp: temp,
    }
}

fn append(sidecar: &std::path::Path, records: &[serde_json::Value]) {
    use std::io::Write;
    std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(sidecar)
        .unwrap();
    for record in records {
        writeln!(file, "{record}").unwrap();
    }
}

fn tool_call(call: u64, tool: &str, head: &str, status: &str) -> serde_json::Value {
    serde_json::json!({"kind": "tool_call", "call": call, "tool": tool, "head": head, "status": status})
}

/// The agent's answer: accepted, the declared tests in `commands_run`, the
/// MCP tools left out, and the artifact claimed at `artifact`.
fn answer(artifact: &str) -> String {
    let mut result = WorkflowV2Result::accepted("remediated on the declared test surface");
    result.commands_run = vec![WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "cargo test -p archon-trading --test stooq_ingest_artifacts".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "7 passed".to_string(),
        pre_existing: false,
    }];
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Artifact,
        artifact,
    ));
    result.artifacts.push(WorkflowV2Artifact {
        id: "remediation".to_string(),
        path: artifact.to_string(),
        description: None,
    });
    serde_json::to_string(&result).unwrap()
}

struct Scripted {
    answers: Mutex<VecDeque<String>>,
    prompts: Mutex<Vec<String>>,
}

impl Scripted {
    fn new(answers: Vec<String>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            prompts: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl WorkflowV2AgentClient for Scripted {
    async fn run_agent(&self, prompt: String) -> Result<String, WorkflowV2AgentError> {
        self.prompts.lock().unwrap().push(prompt);
        Ok(self.answers.lock().unwrap().pop_front().expect("an answer"))
    }
}

/// The live sequence: the host saw all five MCP tools succeed, the report
/// omitted them, and the first answer named a mis-prefixed artifact path.
/// One repair about the path is all it takes; the tools are never raised.
#[tokio::test]
async fn host_observed_tools_are_credited_and_the_path_repair_then_lands() {
    let fx = fixture();
    // An earlier dispatch of the branch, before this one started: not
    // this session's evidence.
    append(&fx.sidecar, &[tool_call(3, TOOLS[4], "", "ok")]);
    let log = HostToolLog::from_now(fx.sidecar.clone());
    let mut records: Vec<_> = TOOLS[..4]
        .iter()
        .enumerate()
        .map(|(n, tool)| tool_call(50 + n as u64, tool, "", "ok"))
        .collect();
    records.push(tool_call(54, TOOLS[4], "", "error: CDP target detached"));
    append(&fx.sidecar, &records);
    let real = fx.project_root.join(&fx.artifact);
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, "{}").unwrap();
    let wrong = fx
        .artifact
        .replace("x-1-69-0-remediation", "remediation-x-1-69-0");
    let client = Scripted::new(vec![answer(&wrong), answer(&fx.artifact)]);

    let result = scope(
        log,
        WorkflowV2AgentAdapter::new().run_with_repair(&client, &fx.request),
    )
    .await
    .expect("the repaired answer stands on the host's record of the tools");
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    let prompts = client.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2);
    assert!(
        prompts[1].contains("do not exist on disk"),
        "{}",
        prompts[1]
    );
    assert!(!prompts[1].contains("never exercised"), "{}", prompts[1]);
}

/// Without the host's record — a report that omits the tools, and a
/// session that never ran them — the first repair turn names BOTH the
/// absent artifact and every unexercised tool, instead of one per turn.
#[tokio::test]
async fn one_repair_turn_names_every_contract_violation() {
    let fx = fixture();
    let log = HostToolLog::from_now(fx.sidecar.clone());
    // The shapes that are NOT an invocation of the tool: a shell command
    // that names it, a refused call, and a call on another server.
    append(
        &fx.sidecar,
        &[
            tool_call(
                1,
                "Bash",
                "echo mcp__tradingview__tv_health_check",
                "exit 0",
            ),
            tool_call(2, TOOLS[1], "", "refused: read budget exhausted"),
            tool_call(3, "mcp__other__tv_ui_state", "", "ok"),
        ],
    );
    let error = scope(log, async {
        WorkflowV2AgentAdapter::new().parse_agent_output(&fx.request, &answer(&fx.artifact))
    })
    .await
    .expect_err("an absent artifact and five unexercised tools");
    let WorkflowV2AgentError::ContractViolations(violations) = &error else {
        panic!("expected every violation at once: {error}");
    };
    assert!(
        violations
            .iter()
            .any(|v| matches!(v, WorkflowV2AgentError::DeclaredArtifactAbsent(_))),
        "{error}"
    );
    assert!(
        violations.iter().any(|v| matches!(v,
            WorkflowV2AgentError::ImplementationAcceptedWithRequiredToolUnexercised(tools)
                if tools.len() == 5)),
        "{error}"
    );
    let text = error.to_string();
    assert!(
        text.starts_with(&format!(
            "the result has {} problems; correct ALL of them in this one answer: (1) the result declares files or artifacts that do not exist",
            violations.len()
        )),
        "{text}"
    );
    assert!(text.contains(") task declares required_tools"), "{text}");
    // Still the Contract class: a second contract error after this turn
    // does not buy another attempt.
    assert!(!error.differs_from(&WorkflowV2AgentError::InvalidResult("v".into())));
}

#[test]
fn all_of_returns_nothing_one_or_the_flattened_list() {
    assert_eq!(WorkflowV2AgentError::all_of(Vec::new()), Ok(()));
    assert_eq!(
        WorkflowV2AgentError::all_of(vec![WorkflowV2AgentError::DeclaredArtifactAbsent(vec![
            "a".into()
        ])]),
        Err(WorkflowV2AgentError::DeclaredArtifactAbsent(vec![
            "a".into()
        ]))
    );
    let nested = WorkflowV2AgentError::all_of(vec![
        WorkflowV2AgentError::ContractViolations(vec![
            WorkflowV2AgentError::ImplementationAcceptedWithoutChanges,
            WorkflowV2AgentError::ImplementationNoopWithoutTaskCoverage,
        ]),
        WorkflowV2AgentError::DeclaredArtifactAbsent(vec!["a".into()]),
    ])
    .unwrap_err();
    assert!(
        matches!(&nested, WorkflowV2AgentError::ContractViolations(all) if all.len() == 3),
        "{nested:?}"
    );
}

/// Outside a dispatch scope nothing is observed, and the proof is the
/// report alone, as before.
#[test]
fn outside_a_scope_the_report_alone_is_the_evidence() {
    let fx = fixture();
    let mut result = WorkflowV2Result::accepted("x");
    result.commands_run = Vec::new();
    assert_eq!(
        super::unexercised_required_tools(&fx.request.input, &result).len(),
        5
    );
}
