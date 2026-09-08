//! Production write-wave seam with scripted agent replies and real Git writes.
use archon_workflow::v2::call_data::v2_agent_request;
use archon_workflow::v2::write::run_write_capable_v2_fanout;
use archon_workflow::*;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
#[derive(Clone, Copy)]
enum Reply {
    Accepted,
    Malformed,
    Timeout,
    Empty,
    MissingCloser,
    SingleQuoteEscape,
}
struct Scripted {
    reply: Reply,
    prompts: Mutex<Vec<String>>,
    resumed: Mutex<bool>,
}
#[async_trait::async_trait]
impl WorkflowAgentDispatch for Scripted {
    fn call_time_budget(&self) -> Option<Duration> {
        Some(Duration::from_secs(1))
    }
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn run_call(
        &self,
        task: &str,
        root: Option<String>,
        execution: &WorkflowV2CallExecution,
        adapter: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        universe: Option<&task_universe::WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        if execution.call.write_mode.is_none() {
            return Ok(WorkflowV2Result::accepted("scope unchanged"));
        }
        let root = PathBuf::from(root.unwrap());
        assert!(
            root.join(".git").is_file(),
            "dispatch must use item worktree"
        );
        let request = v2_agent_request(task, Some(root.display().to_string()), execution, universe);
        self.prompts
            .lock()
            .unwrap()
            .push(adapter.build_prompt(&request));
        *self.resumed.lock().unwrap() = root.join("added.txt").exists();
        std::fs::write(root.join("owned.txt"), "implemented\n").unwrap();
        std::fs::write(root.join("added.txt"), "retained new file\n").unwrap();
        let output=json!({"status":"accepted","summary":"implemented owned files",
            "evidence":[{"kind":"implementation","summary":"changed owned files and checked contents"}],
            "files_changed":[{"path":"owned.txt"},{"path":"added.txt"}],
            "commands_run":[{"kind":"test","command":"test -s owned.txt && test -s added.txt","status":"succeeded","exit_code":0,"output_summary":"both files present"}],
            "task_coverage":[{"task_id":"TASK-001","status":"accepted","summary":"files implemented","evidence":[{"kind":"implementation","summary":"owned files exist"}]}],
            "data":{"canonical_task_ids":["TASK-001"]}
        }).to_string();
        let status = std::process::Command::new("sh")
            .args(["-c", "test -s owned.txt && test -s added.txt"])
            .current_dir(&root)
            .status()
            .unwrap();
        assert!(status.success());
        match self.reply {
            Reply::Timeout => {
                tokio::time::sleep(Duration::from_millis(1100)).await;
                Err(WorkflowError::StageFailed(
                    "agent call timed out after writing files".into(),
                ))
            }
            other => {
                let raw = match other {
                    Reply::Empty => String::new(),
                    Reply::Malformed => "{\"status\":\"accepted\",\"summary\":\"unfinished".into(),
                    Reply::MissingCloser => output[..output.len() - 1].to_string(),
                    Reply::SingleQuoteEscape => {
                        output.replace("implemented owned files", r"implemented owner\'s files")
                    }
                    _ => output,
                };
                let client = RepeatedReply(raw);
                adapter
                    .run_with_repair(&client, &request)
                    .await
                    .map_err(|e| WorkflowError::StageFailed(format!("schema repair failed: {e}")))
            }
        }
    }
}
struct RepeatedReply(String);
#[async_trait::async_trait]
impl archon_workflow::v2::agent_adapter::WorkflowV2AgentClient for RepeatedReply {
    async fn run_agent(
        &self,
        _: String,
    ) -> Result<String, archon_workflow::v2::agent_adapter::WorkflowV2AgentError> {
        Ok(self.0.clone())
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: WorkflowStore,
    v2: WorkflowV2ResultStore,
    run: String,
    base: String,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "fixture"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(repo.join("owned.txt"), "baseline\n").unwrap();
        git(&repo, &["add", "owned.txt"]);
        git(&repo, &["commit", "-qm", "baseline"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        let store = WorkflowStore::project(&temp.path().join("project"));
        let run = store
            .create_run(WorkflowSpec {
                schema: spec::WORKFLOW_SCHEMA.into(),
                name: "write-seam".into(),
                task: "implement files".into(),
                target_repository_root: Some(repo.display().to_string()),
                max_parallelism: 1,
                max_agents: 1,
                stages: vec![],
                permissions: Default::default(),
                learning_hooks: vec![],
            })
            .unwrap();
        let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
        Self {
            _temp: temp,
            repo,
            store,
            v2,
            run: run.id,
            base,
        }
    }
    async fn wave(&self, id: &str, reply: Reply) -> (WorkflowV2Result, Scripted) {
        let dispatch = Scripted {
            reply,
            prompts: Mutex::new(vec![]),
            resumed: Mutex::new(false),
        };
        let out = self.wave_with_dispatch(id, &dispatch).await;
        (out, dispatch)
    }
    async fn wave_with_dispatch(&self, id: &str, dispatch: &dyn WorkflowAgentDispatch) -> WorkflowV2Result {
        self.wave_with_mode(id, dispatch, WorkflowV2WriteMode::Worktree).await
    }
    async fn wave_with_mode(&self, id: &str, dispatch: &dyn WorkflowAgentDispatch, mode: WorkflowV2WriteMode) -> WorkflowV2Result {
        let call = WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(mode),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".into()),
                task: Some("Implement the item now.".into()),
                target_files_from_item: true,
                ..Default::default()
            },
        };
        let mut branch = call.clone();
        branch.id = format!("{id}-0");
        branch.method = WorkflowV2HostMethod::Implementation;
        branch.options.target_files = vec!["owned.txt".into(), "added.txt".into()];
        let items = vec![WorkflowV2FanoutItem::read_only(
            format!("{id}-0"),
            "coder",
            branch,
            json!({"item":{"item_id":"item-1","canonical_task_ids":["TASK-001"],"target_files":["owned.txt","added.txt"],"work_type":"implementation"}}),
        )];
        run_write_capable_v2_fanout(
            "fallback objective",
            Some(self.repo.to_str().unwrap()),
            WorkflowV2CallExecution {
                call,
                input: json!({}),
                depends_on: vec![],
            },
            WorkflowV2AgentAdapter::new(),
            dispatch,
            &self.v2,
            &self.store,
            &self.run,
            true,
            items,
            None,
            None,
        )
        .await
        .unwrap()
    }
}
#[tokio::test]
async fn accepted_write_wave_commits_real_files_before_return() {
    let f = Fixture::new();
    let (out, _) = f.wave("write-accepted", Reply::Accepted).await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_ne!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    assert_eq!(
        git(&f.repo, &["show", "HEAD:added.txt"]),
        "retained new file"
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
}
#[tokio::test]
async fn malformed_reply_preserves_nonempty_patch_and_next_wave_resumes() {
    preserves_and_resumes(Reply::Malformed).await;
}
#[tokio::test]
async fn timeout_after_files_preserves_nonempty_patch_and_next_wave_resumes() {
    preserves_and_resumes(Reply::Timeout).await;
}
async fn preserves_and_resumes(reply: Reply) {
    let f = Fixture::new();
    let (out, _) = f.wave("write-first", reply).await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    let branch =
        f.v2.load_branch_outcome("write-first", "write-first-0")
            .unwrap()
            .unwrap();
    let data = branch.result.unwrap().data;
    let patch = Path::new(
        data["partial_work"]["patch_path"]
            .as_str()
            .expect("partial patch not captured through wave"),
    );
    let bytes = std::fs::read(patch).unwrap();
    assert!(!bytes.is_empty());
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("implemented") && text.contains("retained new file"));
    let (out, dispatch) = f.wave("write-resumed", Reply::Accepted).await;
    assert!(
        *dispatch.resumed.lock().unwrap(),
        "next item workspace did not apply retained patch"
    );
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:added.txt"]),
        "retained new file"
    );
}

#[tokio::test]
async fn missing_final_closer_does_not_discard_completed_work() {
    let f = Fixture::new();
    let (out, _) = f.wave("write-closer", Reply::MissingCloser).await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:added.txt"]),
        "retained new file"
    );
}
#[tokio::test]
async fn single_quote_escape_does_not_discard_completed_work() {
    let f = Fixture::new();
    let (out, _) = f.wave("write-quote", Reply::SingleQuoteEscape).await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
}
#[tokio::test]
async fn budget_and_resumed_patch_reach_actual_rendered_prompt() {
    let f = Fixture::new();
    let (_, first) = f.wave("write-budget", Reply::Timeout).await;
    let prompts = first.prompts.lock().unwrap();
    assert!(prompts[0].contains("Time budget:"));
    assert!(prompts[0].contains("Write the deliverable files first"));
    drop(prompts);
    let (_, next) = f.wave("write-budget-resume", Reply::Accepted).await;
    let prompts = next.prompts.lock().unwrap();
    assert!(prompts[0].contains("has been applied to this workspace"));
    assert!(prompts[0].contains("Implement the item now."));
}

#[test]
fn syntax_tolerance_does_not_invent_missing_values_or_write_evidence() {
    let call = WorkflowV2HostCall {
        id: "write-invalid".into(),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    };
    let request = v2_agent_request(
        "implement",
        None,
        &WorkflowV2CallExecution {
            call,
            input: json!({}),
            depends_on: vec![],
        },
        None,
    );
    let adapter = WorkflowV2AgentAdapter::new();
    for raw in [
        r#"{"status":"accepted","summary":"cut off"#,
        r#"{"status":"accepted","data":{"count":12"#,
        r#"{"status":"accepted","data":{"ready":tru"#,
        r#"{"status":"accepted","summary":"no evidence""#,
        r#"{"status":"accepted","summary":"bad\qescape"}"#,
    ] {
        assert!(
            adapter.parse_agent_output(&request, raw).is_err(),
            "invalid report was accepted: {raw}"
        );
    }
}

#[tokio::test]
async fn empty_reply_after_writes_retains_partial_and_next_wave_resumes() {
    preserves_and_resumes(Reply::Empty).await;
}

#[tokio::test]
async fn repository_audit_duplicate_is_rejected_before_apply_without_expanding_scope() {
    use archon_workflow::repository_audit::{AuditContract, AuditReport, budget::{AuditPolicy, Limit}, runtime::AuditRuntime};
    let f = Fixture::new();
    let audit = AuditRuntime::initialize(f.store.clone(), f.run.clone(), AuditPolicy {
        attempt_timeout_secs: Limit::Finite(60), total_time_secs: Limit::Unlimited,
        unexpected_change_refreshes: Limit::Unlimited,
    }).unwrap();
    let contract = AuditContract { schema_version:1, snapshot:"fixture".into(), declared_paths:vec!["added.txt".into()] };
    let report: AuditReport = serde_json::from_value(json!({"schema_version":1,"snapshot":"fixture","records":[{
        "declared_path":"added.txt","verdict":"exists_elsewhere","equivalents":["owned.txt"],
        "required_action":"wire_or_migrate","reason":"existing behavior"}]})).unwrap();
    audit.update(|s| s.ledger.accept(contract, report)).unwrap();
    let (out, _) = f.wave("audit-duplicate", Reply::Accepted).await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "audit obligation ignored: {out:#?}");
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), f.base, "unexplained duplicate applied");
}

#[path = "support/write_wave_audit_cache.rs"]
mod audit_cache;
