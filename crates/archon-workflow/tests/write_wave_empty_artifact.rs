//! Issue-12: a declared artifact the branch legitimately leaves empty must not
//! fail the wave.
//!
//! Live, a schema-migration branch rewrote an empty v1 registry as an empty v2
//! registry — population was a later task's job — and the authored script had
//! listed that registry under the item's `artifacts:`. The host failed the
//! branch for `exists but holds no records`, every dependent wave was skipped
//! in the same second, and a genuine code fix in the same branch was never
//! captured. This drives the production write-wave seam with real Git writes:
//! the branch edits its owned file, writes the declared artifact as a valid,
//! empty document, and the wave must commit while the emptiness travels with
//! the result as a `review` gap the verifier can read.
use archon_workflow::v2::call_data::v2_agent_request;
use archon_workflow::v2::write::run_write_capable_v2_fanout;
use archon_workflow::*;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

const ARTIFACT: &str = ".archon/data/registry.json";
const EMPTY_DOCUMENT: &str = r#"{"schema":"x","items":[]}"#;
const EMPTINESS_SENTENCE: &str =
    "exists but holds no records: every array and object in it is empty";

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

/// What the branch writes to the declared artifact, or `None` to leave it
/// unwritten.
struct Scripted {
    artifact_body: Option<&'static str>,
    project_root: PathBuf,
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
        store: Option<&WorkflowV2ResultStore>,
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
        // The code change in the worktree, and the artifact under the PROJECT
        // root — a different tree, exactly as live.
        std::fs::write(root.join("owned.txt"), "migrated schema\n").unwrap();
        if let Some(body) = self.artifact_body {
            let artifact = self.project_root.join(ARTIFACT);
            std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
            std::fs::write(artifact, body).unwrap();
        }
        let mut request =
            v2_agent_request(task, Some(root.display().to_string()), execution, universe);
        // What the live host dispatch stamps before the adapter sees the
        // reply: the project artifact context, with the item's declared
        // artifact requirements added to it.
        let store = store.expect("the write seam hands its store to dispatch");
        let mut context = archon_workflow::project_artifact_context_from_v2_root(store.root());
        context.repository_root = request.repository_root.clone();
        context.add_artifact_requirements(&request.input);
        request.project_artifacts = context;
        self.prompts
            .lock()
            .unwrap()
            .push(adapter.build_prompt(&request));
        let envelope = json!({
            "status": "accepted",
            "summary": "migrated the schema; the registry stays empty until a later task populates it",
            "evidence": [{"kind": "implementation", "summary": "rewrote the schema module"}],
            "files_changed": [{"path": "owned.txt"}],
            "commands_run": [{"kind": "test", "command": "test -s owned.txt", "status": "succeeded", "exit_code": 0, "output_summary": "ok"}],
            "task_coverage": [{"task_id": "TASK-001", "status": "accepted", "summary": "schema migrated",
                "evidence": [{"kind": "implementation", "summary": "owned file rewritten"}]}],
            "data": {"canonical_task_ids": ["TASK-001"]}
        });
        let client = Reply(envelope.to_string());
        adapter
            .run_with_repair(&client, &request)
            .await
            .map_err(|e| WorkflowError::StageFailed(format!("schema repair failed: {e}")))
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

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    project: PathBuf,
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
        let project = temp.path().join("project");
        let store = WorkflowStore::project(&project);
        let run = store
            .create_run(WorkflowSpec {
                schema: spec::WORKFLOW_SCHEMA.into(),
                name: "empty-artifact".into(),
                task: "migrate the schema".into(),
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
            project,
            store,
            v2,
            run: run.id,
            base,
        }
    }

    /// One wave of one branch `<id>-0` that declares `ARTIFACT` and writes
    /// `artifact_body` to it.
    async fn wave(&self, id: &str, artifact_body: Option<&'static str>) -> WorkflowV2Result {
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
        let branch_id = format!("{id}-0");
        let mut branch = call.clone();
        branch.id = branch_id.clone();
        branch.method = WorkflowV2HostMethod::Implementation;
        branch.options.target_files = vec!["owned.txt".into()];
        let items = vec![WorkflowV2FanoutItem::read_only(
            branch_id.clone(),
            "coder",
            branch,
            json!({"item": {"item_id": branch_id, "canonical_task_ids": ["TASK-001"],
                "target_files": ["owned.txt"], "work_type": "implementation",
                "artifact_requirements": [{"path": ARTIFACT}]}}),
        )];
        let dispatch = Scripted {
            artifact_body,
            project_root: self.project.clone(),
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
            items,
            None,
            None,
        )
        .await
        .unwrap()
    }

    fn branch_result(&self, call_id: &str, branch_id: &str) -> WorkflowV2Result {
        self.v2
            .load_branch_outcome(call_id, branch_id)
            .unwrap()
            .unwrap()
            .result
            .unwrap()
    }

    fn manifest_path(&self, call_id: &str, branch_id: &str) -> PathBuf {
        self.store.run_dir(&self.run).join(format!(
            "write-coordination/stages/{call_id}/manifests/{branch_id}.json"
        ))
    }
}

fn empty_artifact_gap(result: &WorkflowV2Result, branch_id: &str) -> Option<WorkflowV2ResidualGap> {
    result
        .residual_gaps
        .iter()
        .find(|gap| gap.id == format!("artifact_structurally_empty_{branch_id}"))
        .cloned()
}

/// The live shape in miniature: the wave commits the code change, the
/// manifest is persisted, and the emptiness is carried as a `review` gap on
/// the branch result, in the fan-out's `data.items`, and on the aggregate.
#[tokio::test]
async fn an_empty_declared_artifact_commits_the_wave_and_carries_a_review_gap() {
    let f = Fixture::new();
    let out = f.wave("empty", Some(EMPTY_DOCUMENT)).await;

    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_ne!(
        git(&f.repo, &["rev-parse", "HEAD"]),
        f.base,
        "the wave must commit"
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "migrated schema");
    assert!(
        f.manifest_path("empty", "empty-0").is_file(),
        "manifest persisted for the accepted branch"
    );
    assert_eq!(
        std::fs::read_to_string(f.project.join(ARTIFACT)).unwrap(),
        EMPTY_DOCUMENT,
        "the artifact is left exactly as the branch wrote it"
    );

    let branch = f.branch_result("empty", "empty-0");
    assert_eq!(branch.status, WorkflowV2Status::Accepted, "{branch:#?}");
    assert!(
        !branch
            .summary
            .contains("declared project artifacts missing"),
        "{}",
        branch.summary
    );
    assert!(
        branch.data.get("missing_required_artifacts").is_none(),
        "{:#?}",
        branch.data
    );
    let gap = empty_artifact_gap(&branch, "empty-0")
        .unwrap_or_else(|| panic!("no review gap on the branch: {:#?}", branch.residual_gaps));
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(gap.description.contains(ARTIFACT), "{}", gap.description);
    assert!(
        gap.description.contains(EMPTINESS_SENTENCE),
        "{}",
        gap.description
    );
    assert!(
        branch.evidence.iter().any(|evidence| {
            evidence.kind == WorkflowV2EvidenceKind::Review
                && evidence.summary.contains(ARTIFACT)
                && evidence.summary.contains(EMPTINESS_SENTENCE)
        }),
        "{:#?}",
        branch.evidence
    );
    assert!(
        branch
            .artifacts
            .iter()
            .any(|artifact| artifact.path == ARTIFACT),
        "the artifact is recorded as delivered: {:#?}",
        branch.artifacts
    );

    // The authored script reads the branch through `data.items`; the gap
    // must be there, and lifted onto the aggregate the final report reads.
    let items = out.data["items"].as_array().expect("data.items");
    assert!(
        items
            .iter()
            .any(|item| item["residual_gaps"].as_array().is_some_and(|gaps| {
                gaps.iter().any(|gap| {
                    gap["id"] == "artifact_structurally_empty_empty-0"
                        && gap["severity"] == "review"
                })
            })),
        "{items:#?}"
    );
    assert!(
        empty_artifact_gap(&out, "empty-0").is_some(),
        "{:#?}",
        out.residual_gaps
    );
    assert!(
        !out.residual_gaps
            .iter()
            .any(|gap| gap.severity.as_deref() == Some("failed")),
        "{:#?}",
        out.residual_gaps
    );
}

/// The contrast, unchanged: a declared artifact the branch never wrote still
/// fails the branch and nothing is committed.
#[tokio::test]
async fn a_missing_declared_artifact_still_fails_the_wave() {
    let f = Fixture::new();
    let out = f.wave("missing", None).await;

    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["rev-parse", "HEAD"]),
        f.base,
        "nothing may be committed"
    );
    assert!(!f.manifest_path("missing", "missing-0").is_file());
    let branch = f.branch_result("missing", "missing-0");
    assert_eq!(branch.status, WorkflowV2Status::Failed, "{branch:#?}");
    assert!(
        branch
            .summary
            .contains("declared project artifacts missing")
            && branch.summary.contains(ARTIFACT),
        "{}",
        branch.summary
    );
    assert!(
        branch
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "missing_declared_artifacts_missing-0"),
        "{:#?}",
        branch.residual_gaps
    );
    assert!(
        empty_artifact_gap(&branch, "missing-0").is_none(),
        "{:#?}",
        branch.residual_gaps
    );
}

/// A populated artifact carries no review gap: the signal is for emptiness.
#[tokio::test]
async fn a_populated_declared_artifact_carries_no_review_gap() {
    let f = Fixture::new();
    let out = f
        .wave(
            "populated",
            Some(r#"{"schema":"x","items":[{"id":"one"}]}"#),
        )
        .await;

    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_ne!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    let branch = f.branch_result("populated", "populated-0");
    assert_eq!(branch.status, WorkflowV2Status::Accepted, "{branch:#?}");
    assert!(
        empty_artifact_gap(&branch, "populated-0").is_none(),
        "{:#?}",
        branch.residual_gaps
    );
    assert!(
        empty_artifact_gap(&out, "populated-0").is_none(),
        "{:#?}",
        out.residual_gaps
    );
}
