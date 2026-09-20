//! Obs-31: the base-commit baseline over a real sealed worktree — verdicts
//! cached by (commit, command), failures owned by file, the coder told, the
//! scope widened, the verifier's lists persisted.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use archon_write_plan::ForbiddenPaths;
use tokio::sync::Semaphore;

use super::super::test_baseline_wave::{
    BranchBaselineRequest, WaveBaselineContext, establish_wave,
};
use super::super::worktree_scope_grant::ScopeGrant;
use super::super::{
    WorktreeFanoutSetup, WorktreePlanRunContext, prepare_worktree_wave, test_baseline_preamble,
};
use super::{
    all_routed_findings, cached_command, load_record, record_path, routed_findings_for_task,
};
use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::{
    WorkflowV2AgentAdapter, WorkflowV2CallExecution, WorkflowV2FanoutItem, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2ResultStore, WorkflowV2WriteAssignment, WorkflowV2WriteMode,
    WorkflowV2WriteWave,
};
use crate::write_coordinator::WriteCoordinatorConfig;
use crate::write_coordinator::worktree_isolation::run_git;
use crate::{
    WorkflowError, WorkflowResult, WorkflowStore, WorkflowV2FileRecord, WorkflowV2Result,
    WorkflowV2Status,
};

struct Host;
#[async_trait::async_trait]
impl WorkflowAgentDispatch for Host {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        2
    }
    fn baseline_test_timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(2))
    }
    async fn run_call(
        &self,
        _: &str,
        _: Option<String>,
        _: &WorkflowV2CallExecution,
        _: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        _: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        Err(WorkflowError::StageFailed(
            "no agent runs in a baseline test".into(),
        ))
    }
}

fn git(root: &Path, args: &[&str]) {
    run_git(args, root).expect("git");
}

/// A one-package repository (`app`, sources under `src/`) with one commit,
/// and a detached worktree of it at `dir/ws`.
fn repository(dir: &Path) -> (PathBuf, PathBuf) {
    let canonical = dir.join("canonical");
    std::fs::create_dir_all(canonical.join("src")).unwrap();
    std::fs::write(
        canonical.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    for file in ["lib.rs", "mine.rs", "theirs.rs", "nobody.rs"] {
        std::fs::write(canonical.join("src").join(file), "// module\n").unwrap();
    }
    git(&canonical, &["init", "-q"]);
    git(&canonical, &["config", "user.name", "t"]);
    git(&canonical, &["config", "user.email", "t@example.invalid"]);
    git(&canonical, &["add", "."]);
    git(&canonical, &["commit", "-qm", "base"]);
    let ws = dir.join("ws");
    git(
        &canonical,
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            ws.to_str().unwrap(),
            "HEAD",
        ],
    );
    (canonical, ws)
}

fn head(root: &Path) -> String {
    String::from_utf8(run_git(&["rev-parse", "HEAD"], root).unwrap().stdout)
        .unwrap()
        .trim()
        .to_string()
}

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, files: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: files.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-A", &["src/mine.rs"]),
            task("TASK-B", &["src/theirs.rs"]),
        ],
    }
}

/// A declared command that looks like a cargo test of package `app`, counts
/// its runs in `counter`, and fails three tests in three modules.
fn red_command(counter: &Path) -> String {
    format!(
        ": cargo test -p app ; echo run >> \"{}\"; printf 'test mine::tests::one ... FAILED\\ntest theirs::tests::two ... FAILED\\ntest nobody::tests::three ... FAILED\\n'; exit 101",
        counter.display()
    )
}

fn runs(counter: &Path) -> usize {
    std::fs::read_to_string(counter)
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

fn request(
    branch: &str,
    task: &str,
    command: &str,
    ws: &Path,
    targets: &[&str],
) -> BranchBaselineRequest {
    BranchBaselineRequest {
        branch_id: branch.into(),
        task_ids: vec![task.into()],
        commands: vec![command.into()],
        worktree: ws.to_path_buf(),
        targets: targets.iter().map(|t| t.to_string()).collect(),
        forbidden: ForbiddenPaths::default(),
    }
}

#[tokio::test]
async fn failures_are_owned_by_file_persisted_routed_and_served_from_the_cache_next_time() {
    let temp = tempfile::tempdir().unwrap();
    let (canonical, ws) = repository(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let counter = temp.path().join("runs");
    let command = red_command(&counter);
    let base = head(&canonical);
    let universe = universe();
    let ctx = WaveBaselineContext {
        store: &store,
        dispatch: &Host,
        universe: Some(&universe),
        stage_id: "agents-3",
        base_commit: &base,
        parallelism: 2,
    };
    let records = establish_wave(
        &ctx,
        &[request(
            "agents-3-a",
            "TASK-A",
            &command,
            &ws,
            &["src/mine.rs"],
        )],
    )
    .await;
    assert_eq!(runs(&counter), 1);
    let record = &records[0];
    assert_eq!(record.commands[0].failing_tests.len(), 3);
    assert!(!record.commands[0].cached);
    // Own file: obligation. Nobody's file: obligation. TASK-B's file: routed.
    assert_eq!(
        record.must_pass(),
        vec![
            "mine::tests::one".to_string(),
            "nobody::tests::three".to_string()
        ]
    );
    assert_eq!(
        record.obligation_files(),
        vec!["src/mine.rs".to_string(), "src/nobody.rs".to_string()]
    );
    assert_eq!(record.routed.len(), 1);
    assert_eq!(record.routed[0].owner_task, "TASK-B");
    assert_eq!(record.routed[0].file, "src/theirs.rs");
    // Persisted where the verifier stamp and the review merge read it.
    let path = record_path(&store, "agents-3", "agents-3-a");
    assert!(
        path.ends_with("v2/baseline-tests/agents-3/agents-3-a.json"),
        "{}",
        path.display()
    );
    assert_eq!(
        load_record(&store, "agents-3", "agents-3-a").as_ref(),
        Some(record)
    );
    let routed = routed_findings_for_task(&store, "TASK-B");
    assert_eq!(routed.len(), 1);
    assert_eq!(
        routed[0]["canonical_task_ids"],
        serde_json::json!(["TASK-B"])
    );
    assert_eq!(routed[0]["test_id"], "theirs::tests::two");
    assert_eq!(all_routed_findings(&store).len(), 1);
    assert!(cached_command(&store, &base, &command).is_some());

    // Same commit, same command, another stage: nothing runs again, and the
    // owner task's own branch inherits the routed failure as its obligation.
    let ctx = WaveBaselineContext {
        stage_id: "remediate-5",
        ..ctx
    };
    let again = establish_wave(
        &ctx,
        &[
            request("remediate-5-a", "TASK-A", &command, &ws, &["src/mine.rs"]),
            request("remediate-5-b", "TASK-B", &command, &ws, &["src/theirs.rs"]),
        ],
    )
    .await;
    assert_eq!(
        runs(&counter),
        1,
        "cache hit on the same commit and command"
    );
    assert!(again[0].commands[0].cached);
    assert_eq!(again[1].must_pass(), vec!["theirs::tests::two".to_string()]);
    assert!(again[1].routed.iter().all(|r| r.owner_task != "TASK-B"));
    assert_eq!(
        routed_findings_for_task(&store, "TASK-B").len(),
        1,
        "routed once, not per pass"
    );
    // Unowned `src/nobody.rs` was taken by the first branch in the wave; the
    // second is told to ignore that test rather than both declaring the file.
    assert!(
        again[1]
            .routed
            .iter()
            .any(|r| r.test_id == "nobody::tests::three" && r.owner_task == "TASK-A"),
        "{:?}",
        again[1].routed
    );

    // A different commit runs the command again.
    let ctx = WaveBaselineContext {
        base_commit: "ffffffffffffffffffff",
        stage_id: "agents-9",
        ..ctx
    };
    establish_wave(
        &ctx,
        &[request(
            "agents-9-a",
            "TASK-A",
            &command,
            &ws,
            &["src/mine.rs"],
        )],
    )
    .await;
    assert_eq!(runs(&counter), 2);
}

#[tokio::test]
#[cfg(unix)] // Requires Unix process-group teardown, not just leader termination.
async fn a_timed_out_command_is_recorded_without_a_verdict_and_never_cached_or_owed() {
    let temp = tempfile::tempdir().unwrap();
    let (canonical, ws) = repository(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let base = head(&canonical);
    let command = ": cargo test -p app ; sleep 30";
    let ctx = WaveBaselineContext {
        store: &store,
        dispatch: &Host,
        universe: None,
        stage_id: "agents-1",
        base_commit: &base,
        parallelism: 1,
    };
    let started = std::time::Instant::now();
    let records = establish_wave(&ctx, &[request("agents-1-a", "TASK-A", command, &ws, &[])]).await;
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the timeout must end the command"
    );
    let verdict = &records[0].commands[0];
    assert!(verdict.timed_out);
    assert_eq!(verdict.exit_code, None);
    assert_eq!(
        verdict.error.as_deref(),
        Some("baseline command timed out after 2s")
    );
    assert!(records[0].obligations.is_empty());
    assert!(cached_command(&store, &base, command).is_none());
    let text = test_baseline_preamble::preamble(&records[0]);
    assert!(
        text.contains("Declared commands the host could not baseline"),
        "{text}"
    );
}

fn spec() -> crate::WorkflowSpec {
    crate::WorkflowSpec {
        schema: crate::spec::WORKFLOW_SCHEMA.to_string(),
        name: "baseline-test".to_string(),
        task: "test".to_string(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: Vec::new(),
        permissions: std::collections::BTreeMap::new(),
        learning_hooks: Vec::new(),
    }
}

#[tokio::test]
async fn prepare_tells_the_coder_and_widens_its_declared_scope_to_the_obligation_files() {
    let temp = tempfile::tempdir().unwrap();
    let (canonical, _ws) = repository(temp.path());
    let counter = temp.path().join("runs");
    let command = red_command(&counter);
    let control = WorkflowStore::new(temp.path().join("workflows"));
    let run = control.create_run(spec()).unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let universe = universe();
    let execution = WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "agents-3".into(),
            method: WorkflowV2HostMethod::Fanout,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: Default::default(),
        },
        input: serde_json::json!({}),
        depends_on: vec![],
    };
    let setup = WorktreeFanoutSetup {
        canonical_root: canonical.clone(),
        cfg: WriteCoordinatorConfig::default(),
        run_root: temp.path().join("run"),
    };
    let ctx = WorktreePlanRunContext {
        task: "implement",
        target_repository_root: canonical.to_str(),
        execution: &execution,
        adapter: WorkflowV2AgentAdapter::new(),
        dispatch: &Host,
        v2_store: &store,
        store_for_control: &control,
        run_id: &run.id,
        setup: &setup,
        semaphore: Arc::new(Semaphore::new(2)),
        active: Arc::new(AtomicUsize::new(0)),
        peak: Arc::new(AtomicUsize::new(0)),
        task_universe: Some(&universe),
    };
    let branch = WorkflowV2FanoutItem::read_only(
        "agents-3-a",
        "coder",
        WorkflowV2HostCall {
            id: "agents-3-agents-3-a".into(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Worktree),
            options: Default::default(),
        },
        serde_json::json!({"item": {
            "item_id": "agents-3-a",
            "canonical_task_ids": ["TASK-A"],
            "target_files": ["src/mine.rs"],
            "focused_verification": [command],
        }}),
    );
    let wave = WorkflowV2WriteWave {
        assignments: vec![WorkflowV2WriteAssignment {
            item_id: "agents-3-a".into(),
            owned_targets: vec!["src/mine.rs".into()],
            owned_scopes: Vec::new(),
            worktree_path: Some(temp.path().join("iso/agents-3-a").display().to_string()),
            artifact_only: false,
        }],
    };
    let prepared = prepare_worktree_wave(&ctx, &wave, &[branch]).await.unwrap();
    assert_eq!(runs(&counter), 1);
    let branch = &prepared[0];
    let targets: Vec<String> = branch
        .coordinator_plan
        .target_files
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(
        targets,
        vec!["src/mine.rs".to_string(), "src/nobody.rs".to_string()]
    );
    assert!(
        branch
            .assignment
            .owned_targets
            .contains(&"src/nobody.rs".to_string())
    );
    assert!(branch.wave_claims[0].owned.contains("src/nobody.rs"));
    assert!(
        branch
            .baseline
            .declared_target_meta
            .contains_key("src/nobody.rs"),
        "stale recheck covers the widened file"
    );
    let record = branch.test_baseline.as_ref().unwrap();
    assert_eq!(record.routed[0].owner_task, "TASK-B");

    // The widened file survives the grant as a declared target: a fix there
    // is neither out of scope nor an undeclared grant.
    std::fs::write(
        branch.workspace.plan.isolated_root.join("src/nobody.rs"),
        "// fixed\nfn f() {}\n",
    )
    .unwrap();
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        ..Default::default()
    };
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/nobody.rs"));
    let grant = ScopeGrant::resolve_unforbidden(
        &branch.coordinator_plan,
        &result,
        Some(&branch.wave_claims),
    );
    assert!(grant.out_of_scope.is_empty(), "{grant:?}");
    assert!(grant.granted.is_empty(), "declared, not granted: {grant:?}");
    assert!(grant.covers("src/nobody.rs"));

    // Exactly what the coder is told.
    let text = test_baseline_preamble::preamble(record);
    let sha: String = head(&canonical).chars().take(12).collect();
    assert_eq!(
        text,
        format!(
            "\nBaseline tests (the host ran your declared focused test commands on the base commit {sha}, in this worktree, before you started):\n\
             - Tests already failing on the base commit within your declared filter: mine::tests::one (src/mine.rs), nobody::tests::three (src/nobody.rs) — these are yours to make pass; their files are in your scope.\n\
             - Tests already failing on the base commit within your declared filter but owned by another task: theirs::tests::two — owned by TASK-B, ignore. Do not edit their files; they are routed to their owner.\n\
             Your task is not accepted while any test in your declared filter fails, except the ones listed above as owned by another task or to leave alone; \"pre-existing\" is not an acceptable reason, and neither is disabling or deleting the test.\n"
        )
    );
}
