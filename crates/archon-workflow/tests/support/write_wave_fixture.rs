//! The write-wave seam harness: one Git repository, one scripted dispatch
//! that edits a branch's worktree and reports an envelope, and the production
//! `run_write_capable_v2_fanout` driven end to end. Shared by the scope-grant
//! and whitespace-drop tests.
#![allow(dead_code)]
use archon_workflow::v2::call_data::v2_agent_request;
use archon_workflow::v2::write::run_write_capable_v2_fanout;
use archon_workflow::*;
use serde_json::json;
use std::{
    collections::BTreeMap,
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

/// What one branch writes in its worktree and what its envelope then reports.
#[derive(Clone)]
pub struct Edits {
    pub files: Vec<(&'static str, &'static str)>,
    pub report: Vec<&'static str>,
    /// Run the envelope through the real adapter (the live path) or hand the
    /// host a parsed result directly, so the gate under test is the host's own
    /// and not the adapter's earlier copy of the same check.
    pub via_adapter: bool,
}

struct Scripted {
    per_branch: BTreeMap<String, Edits>,
    prompts: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowAgentDispatch for Scripted {
    fn call_time_budget(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }
    fn dispatch_timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(7_200))
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
        _store: Option<&WorkflowV2ResultStore>,
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
        let edits = self.per_branch[&execution.call.id].clone();
        for (path, content) in &edits.files {
            let target = root.join(path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, content).unwrap();
        }
        let request = v2_agent_request(task, Some(root.display().to_string()), execution, universe);
        self.prompts
            .lock()
            .unwrap()
            .push(adapter.build_prompt(&request));
        let files_changed: Vec<_> = edits
            .report
            .iter()
            .map(|path| json!({"path": path}))
            .collect();
        let envelope = json!({
            "status": "accepted",
            "summary": "implemented",
            "evidence": [{"kind": "implementation", "summary": "wrote the files"}],
            "files_changed": files_changed,
            "commands_run": [{"kind": "test", "command": "true", "status": "succeeded", "exit_code": 0, "output_summary": "ok"}],
            "task_coverage": [{"task_id": "TASK-001", "status": "accepted", "summary": "done",
                "evidence": [{"kind": "implementation", "summary": "files exist"}]}],
            "data": {"canonical_task_ids": ["TASK-001"]}
        });
        if edits.via_adapter {
            let client = Reply(envelope.to_string());
            return adapter
                .run_with_repair(&client, &request)
                .await
                .map_err(|e| WorkflowError::StageFailed(format!("schema repair failed: {e}")));
        }
        Ok(serde_json::from_value(envelope).unwrap())
    }
}

struct Reply(String);
#[async_trait::async_trait]
impl archon_workflow::v2::agent_adapter::WorkflowV2AgentClient for Reply {
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

pub const FORMATTED_BASELINE: &str = "fn f() {\n    1\n}\n";

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["config", "user.name", "fixture"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(repo.join("owned.txt"), "baseline\n").unwrap();
        std::fs::write(repo.join("other.txt"), "other baseline\n").unwrap();
        std::fs::create_dir(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/formatted.txt"), FORMATTED_BASELINE).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "baseline"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        let store = WorkflowStore::project(&temp.path().join("project"));
        let run = store
            .create_run(WorkflowSpec {
                schema: spec::WORKFLOW_SCHEMA.into(),
                name: "scope-grant".into(),
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

    /// One wave of `items`: each is (declared targets, edits) and becomes
    /// branch `<id>-<index>`.
    pub async fn wave(&self, id: &str, items: Vec<(Vec<&str>, Edits)>) -> WorkflowV2Result {
        let call = WorkflowV2HostCall {
            id: id.into(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: WorkflowV2HostOptions {
                item_kind: Some("implementation".into()),
                task: Some("Implement the item now.".into()),
                target_files_from_item: true,
                ..Default::default()
            },
        };
        let mut per_branch = BTreeMap::new();
        let mut branches = Vec::new();
        for (index, (targets, edits)) in items.into_iter().enumerate() {
            let branch_id = format!("{id}-{index}");
            let mut branch = call.clone();
            branch.id = branch_id.clone();
            branch.method = WorkflowV2HostMethod::Implementation;
            branch.options.target_files = targets.iter().map(|t| (*t).to_string()).collect();
            branches.push(WorkflowV2FanoutItem::read_only(
                branch_id.clone(),
                "coder",
                branch,
                json!({"item": {"item_id": branch_id, "canonical_task_ids": ["TASK-001"],
                    "target_files": targets, "work_type": "implementation"}}),
            ));
            per_branch.insert(branch_id, edits);
        }
        let dispatch = Scripted {
            per_branch,
            prompts: Mutex::new(vec![]),
        };
        run_write_capable_v2_fanout(
            "fallback objective",
            Some(self.repo.to_str().unwrap()),
            WorkflowV2CallExecution {
                call,
                input: json!({}),
                depends_on: vec![],
            },
            WorkflowV2AgentAdapter::new(),
            &dispatch,
            &self.v2,
            &self.store,
            &self.run,
            true,
            branches,
            None,
            None,
        )
        .await
        .unwrap()
    }

    pub fn branch_result(&self, call_id: &str, branch_id: &str) -> WorkflowV2Result {
        self.v2
            .load_branch_outcome(call_id, branch_id)
            .unwrap()
            .unwrap()
            .result
            .unwrap()
    }

    pub fn manifest(&self, call_id: &str, branch_id: &str) -> serde_json::Value {
        let path = self.store.run_dir(&self.run).join(format!(
            "write-coordination/stages/{call_id}/manifests/{branch_id}.json"
        ));
        serde_json::from_str(&std::fs::read_to_string(&path).expect("manifest persisted")).unwrap()
    }

    pub fn branch_gap(&self, call_id: &str, branch_id: &str) -> String {
        let result = self.branch_result(call_id, branch_id);
        result
            .residual_gaps
            .iter()
            .find(|gap| gap.id == format!("invalid_write_branch_output_{branch_id}"))
            .unwrap_or_else(|| panic!("no ownership gap: {result:#?}"))
            .description
            .clone()
    }
}
