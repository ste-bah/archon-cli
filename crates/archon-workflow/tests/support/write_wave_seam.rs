//! The write-wave end-to-end seam: production `run_write_capable_v2_fanout`
//! with scripted agent replies and real Git writes. Shared by the
//! end-to-end, timeout-retry and repository-audit write-wave tests.
#![allow(dead_code)]
use archon_workflow::v2::call_data::v2_agent_request;
use archon_workflow::v2::write::run_write_capable_v2_fanout;
use archon_workflow::*;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

pub fn git(root: &Path, args: &[&str]) -> String {
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
pub enum Reply {
    Accepted,
    Malformed,
    Timeout,
    /// The host cuts the first session after it wrote its files; the in-run
    /// retry then returns the accepted envelope.
    TimeoutOnce,
    /// The host's own timer cuts EVERY session, typed the way the live
    /// dispatch reports it, and returns at once: only the classification can
    /// stop a re-ask, not the wall clock.
    HostCut,
    /// The host cuts the first session; every later one is a genuine provider
    /// drop, so the retry's own transport re-asks are exercised.
    HostCutThenDrop,
    /// The runner stops every session for making no progress (Issue-213 C2),
    /// after it wrote its files.
    NoProgress,
    Empty,
    MissingCloser,
    SingleQuoteEscape,
}
pub struct Scripted {
    pub reply: Reply,
    pub prompts: Mutex<Vec<String>>,
    pub resumed: Mutex<bool>,
    /// The per-dispatch timeout override each call carried, in call order.
    pub timeout_overrides: Mutex<Vec<Option<u64>>>,
    pub call_budget: Duration,
    pub retry_budget: Duration,
}
#[async_trait::async_trait]
impl WorkflowAgentDispatch for Scripted {
    fn call_time_budget(&self) -> Option<Duration> {
        Some(self.call_budget)
    }
    fn dispatch_timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(7_200))
    }
    fn timeout_retry_budget(&self) -> Option<Duration> {
        Some(self.retry_budget)
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
        store: Option<&WorkflowV2ResultStore>,
        universe: Option<&task_universe::WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        if execution.call.write_mode.is_none() {
            return Ok(WorkflowV2Result::accepted("scope unchanged"));
        }
        // What the guard would have recorded had this session tried a
        // release build: the sidecar the live dispatch scopes per call.
        if let Some(store) = store {
            let sidecar = archon_workflow::v2::write_read_set::path(store, &execution.call.id);
            std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(sidecar)
                .unwrap();
            use std::io::Write;
            writeln!(file, r#"{{"kind":"refusal","call":1,"tool":"Bash","head":"cargo build --release","reason":"Release builds are disabled for this write-capable workflow call."}}"#).unwrap();
            writeln!(file, r#"{{"kind":"tool_call","call":1,"tool":"Bash","head":"cargo build --release","status":"refused: Release builds are disabled for this write-capable workflow call."}}"#).unwrap();
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
        self.timeout_overrides.lock().unwrap().push(
            execution
                .call
                .options
                .extra
                .get(archon_workflow::agent_dispatch_port::DISPATCH_TIMEOUT_OVERRIDE_KEY)
                .and_then(serde_json::Value::as_u64),
        );
        let call_index = self.prompts.lock().unwrap().len();
        *self.resumed.lock().unwrap() = root.join("added.txt").exists();
        // Each session leaves a different edit, so a partial patch says which
        // session it was captured after.
        let owned = if call_index == 1 || matches!(self.reply, Reply::HostCutThenDrop) {
            "implemented\n"
        } else {
            "implemented by retry\n"
        };
        std::fs::write(root.join("owned.txt"), owned).unwrap();
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
            Reply::HostCut | Reply::HostCutThenDrop
                if call_index == 1 || matches!(self.reply, Reply::HostCut) =>
            {
                Err(WorkflowError::HostCallTimeout(
                    "agent transport failed: subagent timed out after 1800s".into(),
                ))
            }
            Reply::HostCutThenDrop => {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Err(WorkflowError::StageFailed(
                    "agent transport failed: subagent failed: HTTP error: response_failed".into(),
                ))
            }
            Reply::NoProgress => Err(WorkflowError::StageFailed(format!(
                "agent transport failed: subagent failed: {} oscillation: the working tree \
                 returned to a state it had already left 3 times in a row",
                archon_tools::NO_PROGRESS_STOP_MARKER
            ))),
            Reply::Timeout | Reply::TimeoutOnce
                if call_index == 1 || matches!(self.reply, Reply::Timeout) =>
            {
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
pub struct Fixture {
    _temp: tempfile::TempDir,
    pub repo: PathBuf,
    pub store: WorkflowStore,
    pub v2: WorkflowV2ResultStore,
    pub run: String,
    pub base: String,
}
impl Fixture {
    pub fn new() -> Self {
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
        let store = WorkflowStore::project(temp.path().join("project"));
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
    pub async fn wave(&self, id: &str, reply: Reply) -> (WorkflowV2Result, Scripted) {
        self.wave_under(
            id,
            reply,
            Duration::from_secs(1),
            Duration::from_secs(1_800),
        )
        .await
    }
    pub async fn wave_under(
        &self,
        id: &str,
        reply: Reply,
        call_budget: Duration,
        retry_budget: Duration,
    ) -> (WorkflowV2Result, Scripted) {
        let dispatch = Scripted {
            reply,
            prompts: Mutex::new(vec![]),
            resumed: Mutex::new(false),
            timeout_overrides: Mutex::new(vec![]),
            call_budget,
            retry_budget,
        };
        let out = self.wave_with_dispatch(id, &dispatch).await;
        (out, dispatch)
    }
    pub async fn wave_with_dispatch(
        &self,
        id: &str,
        dispatch: &dyn WorkflowAgentDispatch,
    ) -> WorkflowV2Result {
        self.wave_with_mode(id, dispatch, WorkflowV2WriteMode::Worktree)
            .await
    }
    pub async fn wave_with_mode(
        &self,
        id: &str,
        dispatch: &dyn WorkflowAgentDispatch,
        mode: WorkflowV2WriteMode,
    ) -> WorkflowV2Result {
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
