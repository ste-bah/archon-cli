use std::path::PathBuf;

use archon_workflow::{
    StageKind, StageRunOutput, StageRunRequest, StageStatus, WorkflowExecutor, WorkflowPolicy,
    WorkflowSpec, WorkflowStageRunner, WorkflowStore,
};

fn permissive_policy() -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    }
}

fn implementation_evidence(task_id: &str, changed_files: &[&str], command: &str) -> StageRunOutput {
    StageRunOutput::markdown(
        serde_json::json!({
            "status": "implemented",
            "implemented_task_ids": [task_id],
            "changed_files": changed_files,
            "commands_run": [{
                "role": "verification",
                "command": command,
                "exit_status": 0
            }],
            "residual_gaps": []
        })
        .to_string(),
    )
}

#[tokio::test]
async fn implementation_fanout_gets_target_repo_sources_and_greenfield_targets() {
    struct EvidenceRunner {
        repo: PathBuf,
        existing_target: PathBuf,
        new_target: PathBuf,
        task_file: PathBuf,
    }

    impl archon_workflow::WriteBoundaryProbe for EvidenceRunner {}
    #[async_trait::async_trait]
    impl WorkflowStageRunner for EvidenceRunner {
        async fn run_stage(
            &self,
            request: StageRunRequest,
        ) -> archon_workflow::WorkflowResult<StageRunOutput> {
            if request.stage_id == "inventory" {
                return Ok(StageRunOutput::markdown(r#"{"items":[]}"#));
            }
            assert_eq!(request.stage_kind, StageKind::Implementation);
            let root = request.input["target_repository_root"].as_str().unwrap();
            assert_eq!(root, self.repo.display().to_string());

            let sources = request.input["source_files"].as_array().unwrap();
            let existing_target = self.existing_target.canonicalize().unwrap();
            let task_file = self.task_file.canonicalize().unwrap();
            assert!(
                sources.iter().any(|source| {
                    source["absolute_path"].as_str() == Some(existing_target.to_str().unwrap())
                        && source["content"].as_str().unwrap().contains("before")
                }),
                "existing target file content should be attached: {sources:#?}"
            );
            assert!(
                sources.iter().any(|source| {
                    source["absolute_path"].as_str() == Some(task_file.to_str().unwrap())
                        && source["content"].as_str().unwrap().contains("REQ-TEST")
                }),
                "task file evidence should be attached: {sources:#?}"
            );
            assert!(
                sources.iter().any(|source| {
                    source["absolute_path"].as_str() == Some(self.new_target.to_str().unwrap())
                        && source["exists"].as_bool() == Some(false)
                }),
                "greenfield target should be attached as exists:false: {sources:#?}"
            );

            std::fs::write(&self.existing_target, "after").unwrap();
            std::fs::write(&self.new_target, "new").unwrap();
            Ok(implementation_evidence(
                "T001",
                &["Cargo.toml", "src/new.rs"],
                "generic verify source evidence fanout",
            ))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let repo = temp.path().join("repo");
    let task_dir = project.join("tasks");
    std::fs::create_dir_all(&task_dir).unwrap();
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();

    let existing_target = repo.join("Cargo.toml");
    let new_target = repo.join("src").join("new.rs");
    let task_file = task_dir.join("TASK.md");
    std::fs::write(&existing_target, "before").unwrap();
    std::fs::write(&task_file, "REQ-TEST: implement the greenfield module").unwrap();

    let spec = WorkflowSpec::from_yaml(&format!(
        r#"
schema: archon.workflow.v1
name: target-repo-source-evidence
task: "Implement {task_file} against repository {repo}"
stages:
  - id: inventory
    kind: agent
    outputs: [items]
  - id: implement_task
    kind: fanout
    item_kind: implementation
    provider_tier: coder
    depends_on: [inventory]
    input:
      items:
        - task_id: T001
          task_file: "{task_file}"
          target_files: ["Cargo.toml", "src/new.rs"]
"#,
        task_file = task_file.display(),
        repo = repo.display()
    ))
    .unwrap();

    let store = WorkflowStore::project(&project);
    let executor = WorkflowExecutor::new(store.clone(), permissive_policy());
    let run = executor.start(spec).unwrap();
    let report = executor
        .execute_with_runner(
            run.clone(),
            &EvidenceRunner {
                repo,
                existing_target,
                new_target,
                task_file,
            },
        )
        .await
        .unwrap();
    assert_eq!(report.failed, 0);

    let state = store.load_state(&run.id).unwrap();
    assert_eq!(
        state.stages.get("implement_task").unwrap().status,
        StageStatus::Accepted
    );
}
