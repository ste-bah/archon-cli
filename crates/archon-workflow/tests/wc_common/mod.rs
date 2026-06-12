//! Shared helpers for TASK-WC-009 coordinated-fanout acceptance tests.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use archon_workflow::fanout::FanoutItem;
use archon_workflow::write_coordinator::coordinator::{
    CoordinatedOutcome, FanoutCtx, FanoutError, PlanInput,
    run_coordinated_implementation_fanout,
};
use archon_workflow::write_coordinator::{
    WriteBoundaryProbe, WriteCoordinatorConfig, resolve_write_coordinator_runtime,
};
use archon_workflow::{
    StageRunOutput, StageRunRequest, WorkflowExecutor, WorkflowPolicy, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

pub type AgentAction = Arc<dyn Fn(&Path, &str, &[String]) -> String + Send + Sync>;

/// A deterministic agent runner: no LLM, no network, no API keys. It writes the
/// declared targets it is redirected to (via `input["target_repository_root"]`)
/// using the provided closure, and reports support for the workspace boundary.
pub struct MockAgentRunner {
    pub action: AgentAction,
    pub max_conc: Option<usize>,
    pub boundary: bool,
}

impl MockAgentRunner {
    pub fn writing(content: &'static str) -> Self {
        Self {
            action: Arc::new(move |root, item, declared| {
                for f in declared {
                    let p = root.join(f);
                    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                    std::fs::write(p, format!("{content}// {item}\n")).unwrap();
                }
                format!("implemented {item}")
            }),
            max_conc: None,
            boundary: true,
        }
    }

    pub fn with_action(action: AgentAction) -> Self {
        Self {
            action,
            max_conc: None,
            boundary: true,
        }
    }

    pub fn cap(mut self, n: usize) -> Self {
        self.max_conc = Some(n);
        self
    }

    pub fn no_boundary(mut self) -> Self {
        self.boundary = false;
        self
    }
}

impl WriteBoundaryProbe for MockAgentRunner {
    fn supports_workspace_boundary(&self) -> bool {
        self.boundary
    }
}

#[async_trait::async_trait]
impl WorkflowStageRunner for MockAgentRunner {
    fn max_concurrency(&self) -> Option<usize> {
        self.max_conc
    }

    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        let root = request.input["target_repository_root"].as_str().unwrap_or(".");
        let declared: Vec<String> = request.input["write_coordination"]["declared_target_files"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let body = (self.action)(Path::new(root), &request.stage_id, &declared);
        Ok(StageRunOutput::markdown(body))
    }
}

pub fn git(args: &[&str], cwd: &Path) {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

/// A canonical git repo with one committed `src/seed.rs`.
pub fn init_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git(&["init", "-q", "-b", "main"], root);
    git(&["config", "user.name", "t"], root);
    git(&["config", "user.email", "t@local"], root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/seed.rs"), "// seed\n").unwrap();
    git(&["add", "-A"], root);
    git(&["commit", "-q", "-m", "init"], root);
    dir
}

/// A non-git directory (for AC-WC-011).
pub fn init_plain_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    dir
}

pub fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

pub fn spec_yaml(canonical: &Path, targets: &[(&str, &[&str])]) -> WorkflowSpec {
    let items: String = targets
        .iter()
        .map(|(name, files)| {
            let list = files
                .iter()
                .map(|f| format!("              - \"{f}\"\n"))
                .collect::<String>();
            format!("        - name: \"{name}\"\n          target_files:\n{list}")
        })
        .collect();
    WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: wc-009
task: coordinated implementation fanout e2e
target_repository_root: "{}"
stages:
  - id: implement
    kind: fanout
    item_kind: implementation
    expected_target_files:
      - "src/a.rs"
    input:
      items:
{items}
"#,
        canonical.display()
    ))
    .unwrap()
}

pub fn plan_inputs(targets: &[(&str, &[&str])]) -> Vec<PlanInput> {
    targets
        .iter()
        .enumerate()
        .map(|(idx, (_n, files))| PlanInput {
            item: FanoutItem {
                id: format!("implement-{idx}"),
                payload: serde_json::json!({ "target_files": files }),
            },
            target_files: files.iter().map(|f| f.to_string()).collect(),
        })
        .collect()
}

/// Full harness output for an acceptance test.
pub struct Harness {
    pub store: WorkflowStore,
    pub run_id: String,
    pub run_root: PathBuf,
    pub outcome: CoordinatedOutcome,
}

/// Drive the coordinator end-to-end against a real repo + cfg, returning the
/// outcome plus the store/run_root so tests can inspect persisted artifacts.
pub fn run_coordinated(
    canonical: &Path,
    targets: &[(&str, &[&str])],
    runner: &dyn WorkflowStageRunner,
    cfg: &WriteCoordinatorConfig,
) -> Result<Harness, FanoutError> {
    let store = WorkflowStore::project(canonical);
    let policy = permissive_policy();
    let executor = WorkflowExecutor::new(store.clone(), policy.clone());
    let run = executor.start(spec_yaml(canonical, targets)).unwrap();
    let run_root = store.run_dir(&run.id);
    let runtime = resolve_write_coordinator_runtime(canonical, cfg);
    let outcome = {
        let stage = run.spec.stages.iter().find(|s| s.id == "implement").unwrap();
        let ctx = FanoutCtx {
            store: &store,
            run: &run,
            policy: &policy,
            stage,
            run_root: run_root.clone(),
            item_deps: BTreeMap::new(),
            verify_inputs: vec![],
        };
        block_on(run_coordinated_implementation_fanout(
            &ctx,
            plan_inputs(targets),
            &runtime,
            cfg,
            runner,
        ))?
    };
    Ok(Harness {
        store,
        run_id: run.id,
        run_root,
        outcome,
    })
}

pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}
