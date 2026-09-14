//! Issue-11: the unclaimed-path grant must reach ownership gate 1.
//!
//! Live on wf-5979fe15 `agents-5`, a single-item wave reported two files it
//! had not declared and nobody else claimed; gate 1 refused the envelope
//! against the DECLARED targets before capture — the only place the grant
//! was applied — ever ran, and eleven dependent waves were skipped. These
//! drive the production write-wave seam with real Git writes and prove the
//! grant is judged once, by every gate, with the same answer.
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

/// What one branch writes in its worktree and what its envelope then reports.
#[derive(Clone)]
struct Edits {
    files: Vec<(&'static str, &'static str)>,
    report: Vec<&'static str>,
    /// Run the envelope through the real adapter (the live path) or hand the
    /// host a parsed result directly, so the gate under test is the host's own
    /// and not the adapter's earlier copy of the same check.
    via_adapter: bool,
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

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    store: WorkflowStore,
    v2: WorkflowV2ResultStore,
    run: String,
    base: String,
}

const FORMATTED_BASELINE: &str = "fn f() {\n    1\n}\n";

impl Fixture {
    fn new() -> Self {
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
    async fn wave(&self, id: &str, items: Vec<(Vec<&str>, Edits)>) -> WorkflowV2Result {
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

    fn branch_result(&self, call_id: &str, branch_id: &str) -> WorkflowV2Result {
        self.v2
            .load_branch_outcome(call_id, branch_id)
            .unwrap()
            .unwrap()
            .result
            .unwrap()
    }

    fn manifest(&self, call_id: &str, branch_id: &str) -> serde_json::Value {
        let path = self.store.run_dir(&self.run).join(format!(
            "write-coordination/stages/{call_id}/manifests/{branch_id}.json"
        ));
        serde_json::from_str(&std::fs::read_to_string(&path).expect("manifest persisted")).unwrap()
    }

    fn branch_gap(&self, call_id: &str, branch_id: &str) -> String {
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

/// (a) The live shape in miniature: one item, one declared file changed, one
/// undeclared file nobody else claims. Gate 1 accepts, capture includes the
/// file, the manifest declares it, and the result says it was granted.
#[tokio::test]
async fn single_item_wave_grants_an_unclaimed_file_through_every_gate() {
    let f = Fixture::new();
    let out = f
        .wave(
            "grant",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        ("forgotten.txt", "also needed\n"),
                    ],
                    report: vec!["owned.txt", "forgotten.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_ne!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    assert_eq!(git(&f.repo, &["show", "HEAD:forgotten.txt"]), "also needed");
    let manifest = f.manifest("grant", "grant-0");
    let declared: Vec<&str> = manifest["declared_target_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(declared.contains(&"forgotten.txt"), "{declared:?}");
    let result = f.branch_result("grant", "grant-0");
    assert_eq!(
        result.data["scope_granted"],
        json!(["forgotten.txt"]),
        "{result:#?}"
    );
    assert!(
        result.evidence.iter().any(
            |e| e.summary.contains("scope granted beyond declared targets")
                && e.summary.contains("forgotten.txt")
        ),
        "{result:#?}"
    );
}

/// (b) The same undeclared file, but the OTHER item in the wave declared it.
/// A contested path is refused exactly as before, by the same message.
#[tokio::test]
async fn two_item_wave_still_rejects_a_file_the_other_item_claims() {
    let f = Fixture::new();
    let out = f
        .wave(
            "contest",
            vec![
                (
                    vec!["owned.txt"],
                    Edits {
                        files: vec![("owned.txt", "implemented\n"), ("other.txt", "trespass\n")],
                        report: vec!["owned.txt", "other.txt"],
                        via_adapter: false,
                    },
                ),
                (
                    vec!["other.txt"],
                    Edits {
                        files: vec![("other.txt", "theirs\n")],
                        report: vec!["other.txt"],
                        via_adapter: false,
                    },
                ),
            ],
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    let gap = f.branch_gap("contest", "contest-0");
    assert_eq!(
        gap,
        "write item 'contest-0' changed undeclared path 'other.txt'"
    );
    assert_ne!(git(&f.repo, &["show", "HEAD:other.txt"]), "trespass");
    let result = f.branch_result("contest", "contest-0");
    assert!(result.data.get("scope_granted").is_none(), "{result:#?}");
}

/// (c) A tree-wide formatter touched a file outside the scope: bytes differ,
/// content minus whitespace does not. Refused, and the message says why.
#[tokio::test]
async fn whitespace_only_change_to_an_undeclared_file_is_refused_by_name() {
    let f = Fixture::new();
    let out = f
        .wave(
            "format",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        ("src/formatted.txt", "fn f() {\n\t1\n}\n\n"),
                    ],
                    report: vec!["owned.txt", "src/formatted.txt"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    let gap = f.branch_gap("format", "format-0");
    assert!(
        gap.contains("changed undeclared path 'src/formatted.txt'"),
        "{gap}"
    );
    assert!(gap.contains("whitespace-only"), "{gap}");
    let result = f.branch_result("format", "format-0");
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "scope_expansion_needed_format-0"
                && gap.description.contains("src/formatted.txt")),
        "{result:#?}"
    );
}

/// (d) The live envelope shape: nine files reported, two undeclared, single
/// item, nothing else claims them. Every gate passes and all nine land.
#[tokio::test]
async fn the_live_nine_file_envelope_with_two_undeclared_paths_lands() {
    let f = Fixture::new();
    let declared = vec![
        "crates/t/src/lib.rs",
        "crates/t/src/validation.rs",
        "crates/t/src/validation/checks.rs",
        "crates/t/src/validation/report.rs",
        "crates/t/tests/native_interval_gates.rs",
        "crates/t/tests/validation_report.rs",
        "crates/t/tests/validation_rules.rs",
    ];
    let undeclared = ["crates/t/src/data_store.rs", "crates/t/src/ohlcv.rs"];
    let files: Vec<(&str, &str)> = declared
        .iter()
        .chain(undeclared.iter())
        .map(|path| (*path, "// implemented\n"))
        .collect();
    let report: Vec<&str> = files.iter().map(|(path, _)| *path).collect();
    assert_eq!(report.len(), 9);
    let out = f
        .wave(
            "live",
            vec![(
                declared.clone(),
                Edits {
                    files,
                    report: report.clone(),
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    for path in &report {
        assert_eq!(
            git(&f.repo, &["show", &format!("HEAD:{path}")]),
            "// implemented",
            "{path}"
        );
    }
    let result = f.branch_result("live", "live-0");
    assert_eq!(
        result.data["scope_granted"],
        json!(undeclared),
        "{result:#?}"
    );
    let manifest = f.manifest("live", "live-0");
    let manifest_declared = manifest["declared_target_files"].to_string();
    for path in undeclared {
        assert!(manifest_declared.contains(path), "{manifest_declared}");
    }
}
