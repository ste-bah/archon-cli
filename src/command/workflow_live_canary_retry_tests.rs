use std::path::PathBuf;
use std::process::Command as GitCommand;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use archon_pipeline::runner::{LlmClient, LlmResponse};
use archon_workflow::CommandAction;

use super::{LiveApprovalMode, run_live_action};

const TASK_ID: &str = "TASK-RETRY-001";
const ARTIFACT_REL: &str = ".archon/artifacts/TASK-RETRY-001/proof.txt";

// tokio's Mutex, not std's: the guard is held for the whole of an async test
// (it serialises ARCHON_SCRIPT_LIFECYCLE mutation), and a std guard held across
// an await point is a deadlock risk clippy rightly rejects.
static LIFECYCLE_ENV_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

struct DecomposedLifecycleEnvGuard {
    previous: Option<String>,
}

impl DecomposedLifecycleEnvGuard {
    async fn set() -> (tokio::sync::MutexGuard<'static, ()>, Self) {
        let guard = LIFECYCLE_ENV_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let previous = std::env::var("ARCHON_SCRIPT_LIFECYCLE").ok();
        unsafe {
            std::env::set_var("ARCHON_SCRIPT_LIFECYCLE", "0");
        }
        (guard, Self { previous })
    }
}

impl Drop for DecomposedLifecycleEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var("ARCHON_SCRIPT_LIFECYCLE", value),
                None => std::env::remove_var("ARCHON_SCRIPT_LIFECYCLE"),
            }
        }
    }
}

struct RetryAgentClient {
    project_root: PathBuf,
    prompts: Mutex<Vec<String>>,
    verification_calls: Mutex<usize>,
    triage_seen: Mutex<bool>,
}

impl RetryAgentClient {
    fn new(project_root: PathBuf) -> Self {
        Self {
            project_root,
            prompts: Mutex::new(Vec::new()),
            verification_calls: Mutex::new(0),
            triage_seen: Mutex::new(false),
        }
    }

    fn proof_path(&self) -> PathBuf {
        self.project_root.join(ARTIFACT_REL)
    }

    fn accepted(summary: &str, data: serde_json::Value) -> String {
        serde_json::json!({
            "status": "accepted",
            "summary": summary,
            "evidence": [{ "kind": "inspection", "summary": summary }],
            "artifacts": [],
            "commands_run": [],
            "files_read": [],
            "files_changed": [],
            "task_coverage": [],
            "residual_gaps": [],
            "data": data
        })
        .to_string()
    }

    fn inventory_item() -> serde_json::Value {
        serde_json::json!({
            "work_type": "implementation",
            "item_id": "impl-retry-001",
            "canonical_task_ids": [TASK_ID],
            "dependency_ids": [],
            "target_files": ["src/lib.rs"],
            "acceptance_criteria": ["Produce proof artifact"],
            "focused_verification": "check proof artifact with schema-aware verifier",
            "artifact_requirements": [ARTIFACT_REL]
        })
    }

    fn verification_item(id: &str) -> serde_json::Value {
        serde_json::json!({
            "item_id": id,
            "canonical_task_ids": [TASK_ID],
            "source_item_id": "impl-retry-001",
            "focused_verification": "run schema-aware proof verification",
            "expected_evidence": "proof artifact accepted by retry verifier",
            "artifact_requirements": [ARTIFACT_REL]
        })
    }

    fn respond(&self, prompt: &str) -> String {
        if prompt.contains("produce dependency-owned inventory items")
            || prompt.contains("Repair inventory shape")
            || prompt.contains("Repair dependency graph defects")
        {
            return Self::accepted(
                "Inventory for retry task.",
                serde_json::json!({ "items": [Self::inventory_item()] }),
            );
        }
        if prompt.contains("Implement only the assigned dependency-ready item") {
            self.write_proof();
            self.write_repo_marker(prompt);
            return self.implementation_result();
        }
        if prompt.contains("Plan focused verification")
            || prompt.contains("Repair an empty focused verification plan")
        {
            return Self::accepted(
                "Verification plan.",
                serde_json::json!({ "items": [Self::verification_item("verify-retry-001")] }),
            );
        }
        if prompt.contains("Run focused verification only")
            || prompt.contains("Run repaired focused verification only")
        {
            return self.verification_result();
        }
        if prompt.contains("Classify failed focused verification outcomes") {
            *self.triage_seen.lock().expect("triage seen") = true;
            let mut retry = Self::verification_item("retry-verify-retry-001");
            retry["source_item_id"] = serde_json::json!("verify-retry-001");
            return Self::accepted(
                "Retry verifier shape.",
                serde_json::json!({
                    "retry_items": [retry],
                    "write_remediation_items": [],
                    "terminal_blockers": []
                }),
            );
        }
        if prompt.contains("Create write-capable remediation items only for actionable") {
            return Self::accepted("No write remediation.", serde_json::json!({ "items": [] }));
        }
        if prompt.contains("List every required generated dataset") {
            return Self::accepted(
                "Artifact inventory.",
                serde_json::json!({ "items": [{ "artifact_path": ARTIFACT_REL }] }),
            );
        }
        Self::accepted(
            "Generic accepted response.",
            serde_json::json!({ "items": [] }),
        )
    }

    fn write_proof(&self) {
        let path = self.proof_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("proof parent");
        }
        std::fs::write(path, "proof\n").expect("proof write");
    }

    fn write_repo_marker(&self, prompt: &str) {
        let Some(root) = repository_root_from_prompt(prompt) else {
            return;
        };
        let path = root.join("src/lib.rs");
        std::fs::write(path, "pub fn proof() -> bool { true }\n").expect("repo marker write");
    }

    fn implementation_result(&self) -> String {
        let artifact_path = self.proof_path().display().to_string();
        serde_json::json!({
            "status": "accepted",
            "summary": "Implemented proof artifact.",
            "evidence": [{ "kind": "implementation", "summary": "proof written" }],
            "artifacts": [{ "id": "proof", "path": artifact_path, "description": "proof" }],
            "commands_run": [{
                "kind": "test",
                "command": "test -f proof.txt",
                "status": "succeeded",
                "exit_code": 0,
                "output_summary": "proof artifact written"
            }],
            "files_read": [],
            "files_changed": [{ "path": "src/lib.rs", "purpose": "proof marker" }],
            "task_coverage": [{
                "task_id": TASK_ID,
                "status": "accepted",
                "summary": "proof written",
                "evidence": [{ "kind": "implementation", "summary": "proof written" }]
            }],
            "residual_gaps": [],
            "data": { "item_id": "impl-retry-001", "canonical_task_ids": [TASK_ID] }
        })
        .to_string()
    }

    fn verification_result(&self) -> String {
        let mut calls = self.verification_calls.lock().expect("verification calls");
        *calls += 1;
        let triaged = *self.triage_seen.lock().expect("triage seen");
        if !triaged {
            return serde_json::json!({
                "status": "needs_review",
                "summary": "Verifier used stale schema expectation.",
                "evidence": [{ "kind": "review", "summary": "test result: failed; failures: stale schema expectation" }],
                "artifacts": [],
                "commands_run": [{
                    "kind": "test",
                    "command": "verify proof artifact",
                    "status": "failed",
                    "exit_code": 1,
                    "output_summary": "test result: failed; failures: stale schema expectation"
                }],
                "files_read": [],
                "files_changed": [],
                "task_coverage": [{
                    "task_id": TASK_ID,
                    "status": "partial",
                    "summary": "verification found stale schema expectation",
                    "evidence": [{ "kind": "test", "summary": "failed verifier expectation" }]
                }],
                "residual_gaps": [{ "id": "stale-verifier", "description": "retry shape" }],
                "data": {
                    "item_id": "verify-retry-001",
                    "canonical_task_ids": [TASK_ID],
                    "matched_test_check_names": { "failed": ["stale schema expectation"] },
                    "pass_fail_count": { "intended_target_passed": 0, "intended_target_failed": 1 },
                    "verification_remediation_required": true,
                    "verification_failure_class": "actionable_implementation_failure",
                    "verification_failure_next_action": "write_remediation"
                }
            })
            .to_string();
        }
        serde_json::json!({
            "status": "accepted",
            "summary": "Retry verification accepted proof artifact.",
            "evidence": [{ "kind": "inspection", "summary": "retry accepted" }],
            "artifacts": [{ "id": "proof", "path": ARTIFACT_REL, "description": "proof" }],
            // The zero-command backstop demotes accepted verifications that
            // executed nothing; an honest retry verifier runs at least the
            // artifact check it claims.
            "commands_run": [{
                "kind": "inspect",
                "command": format!("test -s {ARTIFACT_REL}"),
                "status": "succeeded",
                "exit_code": 0,
                "output_summary": "proof artifact present and non-empty"
            }],
            "files_read": [{ "path": ARTIFACT_REL, "purpose": "proof check" }],
            "files_changed": [],
            "task_coverage": [{
                "task_id": TASK_ID,
                "status": "accepted",
                "summary": "retry verification accepted",
                "evidence": [{ "kind": "inspection", "summary": "retry accepted" }]
            }],
            "residual_gaps": [],
            "data": { "item_id": "retry-verify-retry-001", "canonical_task_ids": [TASK_ID] }
        })
        .to_string()
    }
}

#[async_trait::async_trait]
impl LlmClient for RetryAgentClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        let mut prompt = String::new();
        for value in system.iter().chain(messages.iter()) {
            collect_text(value, &mut prompt);
        }
        let content = self.respond(&prompt);
        self.prompts.lock().expect("prompt log").push(prompt);
        Ok(LlmResponse {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

fn collect_text(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::String(text) => out.push_str(text),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_text(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for item in map.values() {
                collect_text(item, out);
            }
        }
        _ => {}
    }
    out.push('\n');
}

fn repository_root_from_prompt(prompt: &str) -> Option<PathBuf> {
    prompt.lines().find_map(|line| {
        line.strip_prefix("repository_root: ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

fn git(repo: &std::path::Path, args: &[&str]) {
    let output = GitCommand::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git starts");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn triage_retry_items_launch_retry_verification() {
    let (_lifecycle_lock, _lifecycle_env) = DecomposedLifecycleEnvGuard::set().await;
    let (tui_tx, _rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(64);
    let temp = tempfile::tempdir().expect("tempdir");
    let project_root = temp.path();
    let repo = project_root.join("repo");
    std::fs::create_dir_all(repo.join("src")).expect("repo src");
    std::fs::write(repo.join("src/lib.rs"), "pub fn proof() {}\n").expect("seed source");
    git(&repo, &["init"]);
    git(&repo, &["config", "user.name", "retry-canary"]);
    git(&repo, &["config", "user.email", "retry@example.invalid"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "initial"]);

    let tasks = project_root.join("tasks/PRD-RETRY-CANARY");
    std::fs::create_dir_all(&tasks).expect("task dir");
    std::fs::write(
        tasks.join("TASK-RETRY-001-proof.md"),
        super::workflow_live_test_support::standard_task_file(
            TASK_ID,
            "[]",
            "[]",
            &format!(
                "\n## Acceptance Criteria\n\n- Proof artifact exists.\n\n\
                 ## Artifact Requirements\n\n- `{ARTIFACT_REL}`\n"
            ),
        ),
    )
    .expect("task file");

    let task = format!(
        "Implement the decomposed PRD at {} against the repository {}",
        tasks.display(),
        repo.display()
    );
    let client = Arc::new(RetryAgentClient::new(project_root.to_path_buf()));
    let output = run_live_action(
        project_root,
        CommandAction::Run {
            task,
            decomposed: false,
        },
        client.clone(),
        tui_tx,
        None,
        archon_core::config::GeneratedWorkflowConfig::default(),
        true,
        LiveApprovalMode::CliYes,
    )
    .await
    .expect("retry canary completes");

    let verification_calls = *client
        .verification_calls
        .lock()
        .expect("verification call count");
    let prompts = client.prompts.lock().expect("prompts").join("\n---\n");
    assert!(
        verification_calls >= 2,
        "expected initial verification plus triage retry verification; calls={verification_calls}\noutput:\n{output}\nprompts:\n{prompts}"
    );
    assert!(
        prompts.contains("Run repaired focused verification only"),
        "retry prompt missing\noutput:\n{output}\nprompts:\n{prompts}"
    );
    assert!(!output.contains("blocked-verification-failed"), "{output}");
    assert!(
        prompts.contains("final-zero-gap-audit") || output.contains("Workflow V2 complete:"),
        "workflow did not progress beyond verification retry\noutput:\n{output}\nprompts:\n{prompts}"
    );
}
