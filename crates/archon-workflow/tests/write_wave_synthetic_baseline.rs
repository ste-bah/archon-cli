//! Offline replay of the September 4 synthetic workload; not a live-model proof.
use archon_workflow::task_universe::{
    WorkflowV2TaskUniverse, extract_task_universe_for_generated_run,
};
use archon_workflow::v2::call_data::{fanout_items_for_call, v2_agent_request};
use archon_workflow::v2::write::run_write_capable_v2_fanout;
use archon_workflow::*;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
const SCRIPT: &str = include_str!("fixtures/write-wave-synthetic/workflow.js");
fn git(root: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap().trim().into()
}
struct Agent {
    dispatched: Mutex<Vec<String>>,
    fail_seed: bool,
}
#[async_trait::async_trait]
impl WorkflowAgentDispatch for Agent {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn run_call(
        &self,
        task: &str,
        root: Option<String>,
        e: &WorkflowV2CallExecution,
        a: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        universe: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        if e.call.write_mode.is_none() {
            return Ok(WorkflowV2Result::accepted("scope unchanged"));
        }
        let root = PathBuf::from(root.unwrap());
        let id = e.input["item"]["canonical_task_ids"][0].as_str().unwrap();
        self.dispatched.lock().unwrap().push(id.into());
        if self.fail_seed && id == "TASK-SYN-010" {
            return Err(WorkflowError::StageFailed(
                "schema repair failed: malformed output".into(),
            ));
        }
        let task_spec = universe
            .unwrap()
            .tasks
            .iter()
            .find(|t| t.canonical_task_id == id)
            .unwrap();
        let path = task_spec.deliverable_contracts[0].artifact_path.clone();
        if id == "TASK-SYN-020" {
            assert_eq!(
                git(&root, &["show", "HEAD:src/alpha.txt"]),
                "alpha ready",
                "dependent wave must start from seed's committed output"
            );
        }
        std::fs::create_dir_all(root.join(&path).parent().unwrap()).unwrap();
        std::fs::write(
            root.join(&path),
            if id == "TASK-SYN-010" {
                "alpha ready\n"
            } else {
                "{\"ready\":true}\n"
            },
        )
        .unwrap();
        let mut commands = vec![];
        for command in &task_spec.focused_tests {
            let o = std::process::Command::new("sh")
                .args(["-c", command])
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "focused check: {command}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            commands.push(json!({"kind":"test","command":command,"status":"succeeded","exit_code":0,"output_summary":"declared focused test passed"}));
        }
        let request = v2_agent_request(task, Some(root.display().to_string()), e, universe);
        let output = json!({"status":"accepted","summary":"wrote declared deliverable and passed focused tests",
            "evidence":[{"kind":"implementation","summary":"declared deliverable written"}],
            "files_changed":[{"path":path}],"commands_run":commands,
            "task_coverage":[{"task_id":id,"status":"accepted","summary":"focused tests passed","evidence":[{"kind":"implementation","summary":"declared file exists"}]}],
            "data":{}
        });
        a.parse_agent_output(&request, &output.to_string())
            .map_err(|e| WorkflowError::StageFailed(e.to_string()))
    }
}
async fn exercise(fail_seed: bool) {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let tasks = repo.join("tasks/PRD-SYNTHETIC");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::create_dir(repo.join("src")).unwrap();
    for (name, raw) in [
        (
            "TASK-SYN-010.md",
            include_str!("fixtures/write-wave-synthetic/TASK-SYN-010.md"),
        ),
        (
            "TASK-SYN-020.md",
            include_str!("fixtures/write-wave-synthetic/TASK-SYN-020.md"),
        ),
    ] {
        std::fs::write(tasks.join(name), raw).unwrap();
    }
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    git(&repo, &["add", "tasks"]);
    git(&repo, &["commit", "-qm", "synthetic input baseline"]);
    let initial = git(&repo, &["rev-parse", "HEAD"]);
    let universe = extract_task_universe_for_generated_run(&format!(
        "Implement decomposed PRD tasks at {}",
        tasks.display()
    ))
    .unwrap()
    .unwrap();
    assert_eq!(universe.tasks.len(), 2);
    // Execute the original script's QuickJS planning path, then use its actual
    // write-call options with the real source-item builder and write coordinator.
    let planned = archon_workflow::v2::script::dry_run_workflow_plan_full_details(SCRIPT, None)
        .await
        .unwrap();
    let calls = planned
        .calls
        .iter()
        .filter(|c| {
            c.method == WorkflowV2HostMethod::Fanout
                && c.write_mode == Some(WorkflowV2WriteMode::Worktree)
        })
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    let store = WorkflowStore::project(&temp.path().join("project"));
    let run = store
        .create_run(WorkflowSpec {
            schema: spec::WORKFLOW_SCHEMA.into(),
            name: "synthetic-replay".into(),
            task: "synthetic task chain".into(),
            target_repository_root: Some(repo.display().to_string()),
            max_parallelism: 1,
            max_agents: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let agent = Agent {
        dispatched: Mutex::new(vec![]),
        fail_seed,
    };
    for (index, call) in calls.into_iter().enumerate() {
        let task = &universe.tasks[index];
        let item = json!({"item_id":format!("item-{index}"),"canonical_task_ids":[task.canonical_task_id],"target_files":[task.deliverable_contracts[0].artifact_path],"work_type":"implementation"});
        let e = WorkflowV2CallExecution {
            call,
            input: json!({"source_data":[item]}),
            depends_on: vec![],
        };
        let branches = fanout_items_for_call(&e, &v2).unwrap();
        let result = run_write_capable_v2_fanout(
            "implement synthetic chain",
            Some(repo.to_str().unwrap()),
            e,
            WorkflowV2AgentAdapter::new(),
            &agent,
            &v2,
            &store,
            &run.id,
            true,
            branches,
            Some(&universe),
            None,
        )
        .await
        .unwrap();
        if !fail_seed {
            assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
        } else {
            assert_ne!(result.status, WorkflowV2Status::Accepted);
        }
    }
    if fail_seed {
        assert_eq!(*agent.dispatched.lock().unwrap(), vec!["TASK-SYN-010"]);
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), initial);
    } else {
        assert_eq!(
            *agent.dispatched.lock().unwrap(),
            vec!["TASK-SYN-010", "TASK-SYN-020"]
        );
        assert_eq!(
            git(&repo, &["rev-list", "--count", &format!("{initial}..HEAD")]),
            "2"
        );
        assert_eq!(
            git(&repo, &["show", "HEAD:src/beta.json"]),
            "{\"ready\":true}"
        );
        assert!(
            !repo
                .join(".archon/proof/synthetic-observer-target.json")
                .exists()
        );
    }
}
#[tokio::test]
async fn historical_synthetic_waves_commit_and_dispatch_dependent_work() {
    exercise(false).await;
}
#[tokio::test]
async fn failed_seed_holds_dependent_synthetic_wave_without_dispatch() {
    exercise(true).await;
}
