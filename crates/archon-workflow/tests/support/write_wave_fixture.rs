//! The write-wave seam harness: one Git repository, one scripted dispatch
//! that edits a branch's worktree and reports an envelope, and the production
//! `run_write_capable_v2_fanout` driven end to end. Shared by the scope-grant,
//! whitespace-drop and audit-gate tests.
#![allow(dead_code)]
use archon_workflow::repository_audit::AuditContract;
use archon_workflow::repository_audit::budget::{AuditPolicy, Limit};
use archon_workflow::repository_audit::runtime::AuditRuntime;
use archon_workflow::v2::agent_adapter::{WorkflowV2AgentClient, WorkflowV2AgentError};
use archon_workflow::v2::call_data::v2_agent_request;
use archon_workflow::v2::write::run_write_capable_v2_fanout;
use archon_workflow::*;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

pub fn git(root: &Path, args: &[&str]) -> String {
    let mut git = std::process::Command::new("git");
    let out = run_ok(git.arg("-C").arg(root).args(args));
    String::from_utf8(out.stdout).unwrap().trim().into()
}

/// Run `command` to success, returning its output.
pub fn run_ok(command: &mut std::process::Command) -> std::process::Output {
    let out = command.output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    out
}

#[path = "write_wave_paths.rs"]
mod paths;
// Shared by every write-wave test binary; each one uses only some of these.
#[allow(unused_imports)]
pub use paths::{contains_path_text, native, shell_path, toolchain_path};

/// An edit's content that deletes the file instead of writing it.
pub const DELETE: &str = "\u{0}delete";
/// An edit's content prefix that runs the rest as a shell command in the worktree.
pub const RUN: &str = "\u{0}run:";
#[path = "write_wave_guarded.rs"]
mod guarded;
use guarded::guarded_bash;
// Shared by every write-wave test binary; each one uses only some of these.
#[allow(unused_imports)]
pub use guarded::{BASH, COPY};

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

/// A scripted repository audit: which declared paths the audit flags as
/// `exists_elsewhere` / `wire_or_migrate` (with their equivalents), and the
/// dispositions each branch returns for them (declared path, evidence paths).
/// The snapshot is filled in from the runtime at dispatch time, the
/// explanation is fixed.
pub struct AuditScript {
    pub flagged: Vec<(&'static str, Vec<&'static str>)>,
    pub dispositions: BTreeMap<String, Vec<(&'static str, Vec<&'static str>)>>,
}

struct Scripted {
    per_branch: BTreeMap<String, Edits>,
    prompts: Mutex<Vec<String>>,
    /// Shared with the fixture: the host's tool-guard stamps per branch.
    stamps: std::sync::Arc<Mutex<BTreeMap<String, serde_json::Value>>>,
    audit: Option<(AuditRuntime, AuditScript)>,
    /// Branches that write their files and then end `failed` with no
    /// manifest — the shape that leaves partial work behind.
    failing: BTreeSet<String>,
    /// The canonical task ids every item and envelope names.
    task_ids: Vec<String>,
    /// A wave that must replay: any work dispatch is a test failure.
    panic_on_work: bool,
    /// Branches that write their files and then return evidence the host
    /// refuses (a contract failure: needs_review with a result).
    rejecting: BTreeSet<String>,
    /// Shared with the fixture: what each guarded shell command printed.
    shell: std::sync::Arc<Mutex<Vec<String>>>,
}

#[path = "write_wave_audit_script.rs"]
mod audit_script;

#[async_trait::async_trait]
impl WorkflowAgentDispatch for Scripted {
    fn repository_audit(&self) -> Option<AuditRuntime> {
        self.audit.as_ref().map(|(runtime, _)| runtime.clone())
    }
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
        if let Some(contract) = execution
            .call
            .options
            .extra
            .get("repository_audit_contract")
        {
            let contract: AuditContract = serde_json::from_value(contract.clone())?;
            return Ok(self.audit_records(&PathBuf::from(root.unwrap()), &contract));
        }
        if execution.call.write_mode.is_none() {
            return Ok(WorkflowV2Result::accepted("scope unchanged"));
        }
        assert!(
            !self.panic_on_work,
            "{} was dispatched to an agent; it should have replayed",
            execution.call.id
        );
        let root = PathBuf::from(root.unwrap());
        assert!(
            root.join(".git").is_file(),
            "dispatch must use item worktree"
        );
        let edits = self.per_branch[&execution.call.id].clone();
        for (path, content) in &edits.files {
            let target = guarded::edit_target(&root, path, &execution.input, _store);
            if let Some(content) = content.strip_prefix(guarded::WRITE) {
                let run_root = _store.unwrap().run_root();
                let refused =
                    guarded::guarded_write(&root, run_root, &execution.input, &target, content);
                self.shell.lock().unwrap().extend(refused);
                continue;
            }
            if let Some(command) = content.strip_prefix(BASH) {
                // `{copy}` names the branch's copy of a `@copy:` path.
                let command = command.replace("{copy}", &target.display().to_string());
                let run_root = _store.unwrap().run_root();
                let printed = guarded_bash(&root, run_root, &execution.input, &command).await;
                self.shell.lock().unwrap().push(printed);
                continue;
            }
            if *content == DELETE {
                let _ = std::fs::remove_file(target);
                continue;
            }
            if let Some(command) = content.strip_prefix(RUN) {
                let mut sh = std::process::Command::new("sh");
                run_ok(sh.args(["-c", command]).current_dir(&root));
                continue;
            }
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, content).unwrap();
        }
        let mut request =
            v2_agent_request(task, Some(root.display().to_string()), execution, universe);
        guarded::live_artifact_context(&mut request, _store);
        // The guard stamps are read from the input by the dispatch, never rendered
        // into the prompt: kept apart so a prompt assertion cannot match a stamp.
        let stamps: serde_json::Map<String, serde_json::Value> = [
            agent_dispatch_port::FORBIDDEN_PATHS_INPUT_KEY,
            agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY,
            agent_dispatch_port::GRANTABLE_SCOPE_INPUT_KEY,
            agent_dispatch_port::ISOLATED_WORKTREE_INPUT_KEY,
            agent_dispatch_port::WRITE_BOUNDARY_INPUT_KEY,
        ]
        .into_iter()
        .filter_map(|key| Some((key.to_string(), request.input.get(key)?.clone())))
        .collect();
        self.stamps
            .lock()
            .unwrap()
            .insert(execution.call.id.clone(), serde_json::Value::Object(stamps));
        self.prompts
            .lock()
            .unwrap()
            .push(adapter.build_prompt(&request));
        if self.rejecting.contains(&execution.call.id) {
            return Err(WorkflowError::StageFailed(
                "schema repair failed: scripted evidence names no command output".into(),
            ));
        }
        if self.failing.contains(&execution.call.id) {
            return Ok(serde_json::from_value(json!({
                "status": "failed",
                "summary": "scripted: ran out of budget after writing",
                "evidence": [{"kind": "implementation", "summary": "wrote the files, did not finish"}],
                "data": {"canonical_task_ids": self.task_ids, "failure_kind": "execution"}
            }))
            .unwrap());
        }
        let files_changed: Vec<_> = edits
            .report
            .iter()
            .map(|path| json!({"path": path}))
            .collect();
        let noop = edits.files.is_empty() && edits.report.is_empty(); // a typed no-op
        let envelope = json!({
            "status": if noop { "noop" } else { "accepted" },
            "summary": if noop { "already resolved on this tree" } else { "implemented" },
            "evidence": [{"kind": "implementation", "summary": "wrote the files"}],
            "files_changed": files_changed,
            "commands_run": [{"kind": "test", "command": "true", "status": "succeeded", "exit_code": 0, "output_summary": "ok"}],
            "task_coverage": self.task_ids.iter().map(|task| json!({"task_id": task, "status": if noop { "noop" } else { "accepted" }, "summary": "done",
                "evidence": [{"kind": if noop { "inspection" } else { "implementation" }, "summary": "files exist"}]})).collect::<Vec<_>>(),
            "data": {"canonical_task_ids": self.task_ids,
                "audit_dispositions": self.dispositions_for(&execution.call.id)}
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
impl WorkflowV2AgentClient for Reply {
    async fn run_agent(&self, _: String) -> Result<String, WorkflowV2AgentError> {
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
    /// The authoritative task universe a wave runs under, when a test needs
    /// per-task declarations (Issue-30: `files_forbidden_to_change`).
    pub universe: Option<task_universe::WorkflowV2TaskUniverse>,
    /// The canonical task ids each wave item names; one task by default, two
    /// or more for a cross-task item.
    pub item_task_ids: Vec<String>,
    /// The host's tool-guard input stamps each dispatched branch carried,
    /// by branch call id.
    pub stamps: std::sync::Arc<Mutex<BTreeMap<String, serde_json::Value>>>,
    /// What each guarded shell command (`BASH`) printed, in order.
    pub shell: std::sync::Arc<Mutex<Vec<String>>>,
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
        let store = WorkflowStore::project(temp.path().join("project"));
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
            universe: None,
            item_task_ids: vec!["TASK-001".to_string()],
            stamps: Default::default(),
            shell: Default::default(),
        }
    }

    /// The `key` stamp the dispatched branch `call_id` carried in its input:
    /// that branch, or the one branch dispatched under a call of that id.
    pub fn input_stamp(&self, call_id: &str, key: &str) -> serde_json::Value {
        let stamps = self.stamps.lock().unwrap();
        let mut of = stamps.iter().filter(|(id, _)| {
            *id == call_id || stamps.get(call_id).is_none() && id.starts_with(call_id)
        });
        let (_, stamp) = of
            .next()
            .unwrap_or_else(|| panic!("{call_id} not dispatched: {:?}", stamps.keys()));
        assert!(of.next().is_none(), "{call_id} names more than one branch");
        stamp
            .get(key)
            .unwrap_or_else(|| panic!("{call_id} carried no {key}"))
            .clone()
    }

    /// One wave of `items`: each is (declared targets, edits) and becomes
    /// branch `<id>-<index>`.
    pub async fn wave(&self, id: &str, items: Vec<(Vec<&str>, Edits)>) -> WorkflowV2Result {
        self.wave_audited(id, items, None).await.0
    }

    /// The audit runtime for this run, unlimited budgets.
    pub fn audit_runtime(&self) -> AuditRuntime {
        AuditRuntime::initialize(
            self.store.clone(),
            self.run.clone(),
            AuditPolicy {
                attempt_timeout_secs: Limit::Unlimited,
                total_time_secs: Limit::Unlimited,
                unexpected_change_refreshes: Limit::Unlimited,
            },
        )
        .unwrap()
    }

    /// `wave`, with a scripted repository audit when `audit` is given, and
    /// every prompt the branches were sent.
    pub async fn wave_audited(
        &self,
        id: &str,
        items: Vec<(Vec<&str>, Edits)>,
        audit: Option<AuditScript>,
    ) -> (WorkflowV2Result, Vec<String>) {
        self.wave_scripted(id, items, audit, &[]).await
    }

    /// `wave_audited`, with the branches in `failing` ending `failed` after
    /// their edits (their worktree work becomes partial work).
    pub async fn wave_scripted(
        &self,
        id: &str,
        items: Vec<(Vec<&str>, Edits)>,
        audit: Option<AuditScript>,
        failing: &[&str],
    ) -> (WorkflowV2Result, Vec<String>) {
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
        let mut branches = Vec::new();
        for (index, (targets, edits)) in items.into_iter().enumerate() {
            let branch_id = format!("{id}-{index}");
            let mut branch = call.clone();
            branch.id = branch_id.clone();
            branch.method = WorkflowV2HostMethod::Implementation;
            branch.options.target_files = targets.iter().map(|t| (*t).to_string()).collect();
            let item = WorkflowV2FanoutItem::read_only(
                branch_id.clone(),
                "coder",
                branch,
                json!({"item": {"item_id": branch_id, "canonical_task_ids": self.item_task_ids,
                    "target_files": targets, "work_type": "implementation"}}),
            );
            branches.push((item, edits));
        }
        self.wave_on(&self.v2, call, branches, (audit, failing, &[]), false)
            .await
    }

    /// Drive `call` over prepared `branches` through `store` -- a separate
    /// instance is a separate session. With `panic_on_work`, any work
    /// dispatch fails the test: the wave must replay.
    pub async fn wave_on(
        &self,
        store: &WorkflowV2ResultStore,
        call: WorkflowV2HostCall,
        branches: Vec<(WorkflowV2FanoutItem, Edits)>,
        judged: (Option<AuditScript>, &[&str], &[&str]),
        panic_on_work: bool,
    ) -> (WorkflowV2Result, Vec<String>) {
        let task_ids = self.item_task_ids.clone();
        self.wave_for(store, call, branches, judged, task_ids, panic_on_work)
            .await
    }

    /// `wave_on`, with the branch envelopes naming `task_ids` rather than
    /// the fixture's own: a cross-task or escalated item's tasks.
    pub async fn wave_for(
        &self,
        store: &WorkflowV2ResultStore,
        call: WorkflowV2HostCall,
        branches: Vec<(WorkflowV2FanoutItem, Edits)>,
        (audit, failing, rejecting): (Option<AuditScript>, &[&str], &[&str]),
        task_ids: Vec<String>,
        panic_on_work: bool,
    ) -> (WorkflowV2Result, Vec<String>) {
        let per_branch = branches
            .iter()
            .map(|(item, edits)| (item.id.clone(), edits.clone()))
            .collect();
        let branches = branches.into_iter().map(|(item, _)| item).collect();
        let dispatch = Scripted {
            per_branch,
            prompts: Mutex::new(vec![]),
            stamps: self.stamps.clone(),
            audit: audit.map(|script| (self.audit_runtime(), script)),
            failing: failing.iter().map(|id| (*id).to_string()).collect(),
            task_ids,
            panic_on_work,
            rejecting: rejecting.iter().map(|id| (*id).to_string()).collect(),
            shell: self.shell.clone(),
        };
        let result = run_write_capable_v2_fanout(
            "fallback objective",
            Some(self.repo.to_str().unwrap()),
            WorkflowV2CallExecution {
                call,
                input: json!({}),
                depends_on: vec![],
            },
            WorkflowV2AgentAdapter::new(),
            &dispatch,
            store,
            &self.store,
            &self.run,
            true,
            branches,
            self.universe.as_ref(),
            None,
        )
        .await
        .unwrap();
        let prompts = dispatch.prompts.into_inner().unwrap();
        (result, prompts)
    }

    pub fn branch_result(&self, call_id: &str, branch_id: &str) -> WorkflowV2Result {
        let outcome = self.v2.load_branch_outcome(call_id, branch_id).unwrap();
        outcome.unwrap().result.unwrap()
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
