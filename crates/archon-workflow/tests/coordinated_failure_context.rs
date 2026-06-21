use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use archon_workflow::{
    RunStatus, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

fn git(args: &[&str], cwd: &Path) {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn canonical_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "user.name", "t"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "pub fn before() {}\n").unwrap();
    git(&["add", "-A"], root);
    git(&["commit", "-q", "-m", "init"], root);
    dir
}

fn policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

struct FailedVerificationRunner {
    remediation_saw_evidence: Arc<AtomicBool>,
}

impl archon_workflow::WriteBoundaryProbe for FailedVerificationRunner {
    fn supports_workspace_boundary(&self) -> bool {
        true
    }
}

#[async_trait::async_trait]
impl WorkflowStageRunner for FailedVerificationRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        match request.stage_id.as_str() {
            item if item.starts_with("implement-") => {
                let root = request.input["target_repository_root"].as_str().unwrap();
                std::fs::write(
                    Path::new(root).join("src/a.rs"),
                    "pub fn after() { println!(\"changed\"); }\n",
                )
                .unwrap();
                Ok(StageRunOutput::markdown(
                    r#"{"body":{"status":"accepted","work_unit_ids":["one"],"changed_files":["src/a.rs"],"evidence":[{"kind":"file","path":"src/a.rs","summary":"Updated implementation for unit one"}],"residual_gaps":[]},"clippy":{"status":"failed","reason":"cargo clippy -p archon-workflow --tests -- -D warnings failed"}}"#,
                ))
            }
            "remediation-inventory" => {
                let content = request.input["dependencies"][0]["artifacts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|artifact| artifact["content"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(content.contains(r#""clippy""#), "{content}");
                assert!(content.contains("cargo clippy"), "{content}");
                assert!(content.contains("## Agent Output"), "{content}");
                assert!(content.contains("## Patch"), "{content}");
                self.remediation_saw_evidence.store(true, Ordering::SeqCst);
                Ok(StageRunOutput::markdown(r#"{"items":[]}"#))
            }
            _ => Ok(StageRunOutput::markdown("status: completed")),
        }
    }
}

#[tokio::test]
async fn coordinated_verification_failure_feeds_repair_context_and_keeps_run_failed() {
    let canonical = canonical_repo();
    let store = WorkflowStore::project(canonical.path());
    let executor = WorkflowExecutor::new(store.clone(), policy());
    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: coordinated-failure-context
task: Surface failed verification as remediation evidence.
target_repository_root: "{}"
stages:
  - id: implement
    kind: fanout
    item_kind: implementation
    expected_target_files:
      - "src/a.rs"
    input:
      items:
        - name: one
          work_unit_id: one
          target_files:
            - "src/a.rs"
  - id: remediation-inventory
    kind: agent
    outputs: [items]
    depends_on: [implement]
  - id: remediate
    kind: fanout
    foreach: "${{remediation-inventory.items}}"
    item_kind: implementation
    allow_empty_items: true
    depends_on: [remediation-inventory]
"#,
        canonical.path().display()
    ))
    .unwrap();
    let run = executor.start(spec).unwrap();
    let saw_evidence = Arc::new(AtomicBool::new(false));
    let runner = FailedVerificationRunner {
        remediation_saw_evidence: saw_evidence.clone(),
    };

    let report = executor
        .execute_with_runner(run.clone(), &runner)
        .await
        .unwrap();

    assert_eq!(report.failed, 1);
    assert!(saw_evidence.load(Ordering::SeqCst));
    let finished = store.load_state(&run.id).unwrap();
    assert_eq!(finished.status, RunStatus::Failed);
    assert_eq!(
        finished.stages.get("implement").unwrap().status,
        StageStatus::ForcedAccepted
    );
    assert_eq!(
        finished.stages.get("remediation-inventory").unwrap().status,
        StageStatus::Accepted
    );
}
