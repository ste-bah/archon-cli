//! TASK-WC-007 — coordinated implementation fanout integration tests.

use std::collections::BTreeMap;
use std::path::Path;

use archon_workflow::fanout::FanoutItem;
use archon_workflow::write_coordinator::coordinator::{
    FanoutCtx, PlanInput, run_coordinated_implementation_fanout,
};
use archon_workflow::write_coordinator::{
    ManifestStatus, SerialFallbackReason, WriteBoundaryProbe, WriteCoordinatorConfig,
    resolve_write_coordinator_runtime,
};
use archon_workflow::{
    StageRunOutput, StageRunRequest, WorkflowExecutor, WorkflowPolicy, WorkflowRun, WorkflowSpec,
    WorkflowStageRunner, WorkflowStore,
};

fn git(args: &[&str], cwd: &Path) {
    let out = std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A runner that writes the declared targets into the isolated workspace it is
/// pointed at via `input["target_repository_root"]`, and supports the workspace
/// boundary so the coordinated path activates.
struct WritingRunner {
    content: String,
    record_inputs: std::sync::Mutex<Vec<serde_json::Value>>,
}

impl WriteBoundaryProbe for WritingRunner {
    fn supports_workspace_boundary(&self) -> bool {
        true
    }
}

#[async_trait::async_trait]
impl WorkflowStageRunner for WritingRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        self.record_inputs
            .lock()
            .unwrap()
            .push(request.input.clone());
        let root = request.input["target_repository_root"].as_str().unwrap();
        let declared = request.input["write_coordination"]["declared_target_files"]
            .as_array()
            .unwrap();
        for f in declared {
            let p = Path::new(root).join(f.as_str().unwrap());
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            // Per-item suffix keeps overlapping-file writes distinct so each
            // wave produces a non-empty patch.
            std::fs::write(p, format!("{}// {}\n", self.content, request.stage_id)).unwrap();
        }
        Ok(StageRunOutput::markdown(format!(
            "implemented {}",
            request.stage_id
        )))
    }
}

/// A boundary-supporting runner that writes OUTSIDE its declared targets,
/// also mutating the canonical repo directly to trip mutation detection.
struct CanonicalMutatingRunner {
    canonical: std::path::PathBuf,
}

impl WriteBoundaryProbe for CanonicalMutatingRunner {
    fn supports_workspace_boundary(&self) -> bool {
        true
    }
}

#[async_trait::async_trait]
impl WorkflowStageRunner for CanonicalMutatingRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        let root = request.input["target_repository_root"].as_str().unwrap();
        std::fs::write(Path::new(root).join("src/a.rs"), "// isolated\n").unwrap();
        // Mutate the canonical declared target behind the coordinator's back.
        std::fs::write(self.canonical.join("src/a.rs"), "// CANONICAL TAMPER\n").unwrap();
        Ok(StageRunOutput::markdown(format!(
            "tampered {}",
            request.stage_id
        )))
    }
}

/// A runner that does NOT support the workspace boundary (default false).
struct NoBoundaryRunner;
impl WriteBoundaryProbe for NoBoundaryRunner {}

#[async_trait::async_trait]
impl WorkflowStageRunner for NoBoundaryRunner {
    async fn run_stage(
        &self,
        request: StageRunRequest,
    ) -> archon_workflow::WorkflowResult<StageRunOutput> {
        Ok(StageRunOutput::markdown(format!(
            "noop {}",
            request.stage_id
        )))
    }
}

fn canonical_repo() -> tempfile::TempDir {
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

fn impl_fanout_spec(canonical: &Path, targets: &[(&str, &[&str])]) -> WorkflowSpec {
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
name: wc-007
task: coordinated implementation fanout
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

fn ctx_for<'a>(
    store: &'a WorkflowStore,
    run: &'a WorkflowRun,
    policy: &'a WorkflowPolicy,
    run_root: std::path::PathBuf,
) -> FanoutCtx<'a> {
    let stage = run
        .spec
        .stages
        .iter()
        .find(|s| s.id == "implement")
        .unwrap();
    FanoutCtx {
        store,
        run,
        policy,
        stage,
        run_root,
        item_deps: BTreeMap::new(),
        verify_inputs: vec![],
    }
}

fn plan_inputs(targets: &[(&str, &[&str])]) -> Vec<PlanInput> {
    targets
        .iter()
        .enumerate()
        .map(|(idx, (_name, files))| PlanInput {
            item: FanoutItem {
                id: format!("implement-{idx}"),
                payload: serde_json::json!({
                    "target_files": files,
                }),
            },
            target_files: files.iter().map(|f| f.to_string()).collect(),
        })
        .collect()
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn two_disjoint_items_apply_in_one_wave() {
    let canonical = canonical_repo();
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"]), ("b", &["src/b.rs"])];
    let store = WorkflowStore::project(canonical.path());
    let policy = WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    };
    let executor = WorkflowExecutor::new(store.clone(), policy.clone());
    let run = executor
        .start(impl_fanout_spec(canonical.path(), targets))
        .unwrap();
    let cfg = WriteCoordinatorConfig::default();
    let runtime = resolve_write_coordinator_runtime(canonical.path(), &cfg);
    let run_root = canonical.path().join(".archon/workflows").join(&run.id);
    let ctx = ctx_for(&store, &run, &policy, run_root);
    let runner = WritingRunner {
        content: "// written\n".into(),
        record_inputs: std::sync::Mutex::new(vec![]),
    };
    let outcome = block_on(run_coordinated_implementation_fanout(
        &ctx,
        plan_inputs(targets),
        &runtime,
        &cfg,
        &runner,
    ))
    .expect("coordinated fanout");
    assert!(outcome.serial_fallback.is_none());
    assert_eq!(outcome.waves.len(), 1, "disjoint items share one wave");
    assert_eq!(
        outcome.item_status.get("implement-0"),
        Some(&ManifestStatus::Applied)
    );
    assert_eq!(
        outcome.item_status.get("implement-1"),
        Some(&ManifestStatus::Applied)
    );
    assert!(
        std::fs::read_to_string(canonical.path().join("src/a.rs"))
            .unwrap()
            .contains("// written")
    );
    assert!(
        std::fs::read_to_string(canonical.path().join("src/b.rs"))
            .unwrap()
            .contains("// written")
    );
    // input rewrite assertions.
    let inputs = runner.record_inputs.lock().unwrap();
    let any_isolated = inputs.iter().any(|v| {
        v["target_repository_root"]
            .as_str()
            .is_some_and(|s| s.contains(".archon"))
            && v["write_coordination"]["enabled"] == serde_json::json!(true)
    });
    assert!(
        any_isolated,
        "input must carry isolated root + write_coordination.enabled"
    );
}

#[test]
fn overlapping_items_serialize_into_two_waves() {
    let canonical = canonical_repo();
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"]), ("b", &["src/a.rs"])];
    let store = WorkflowStore::project(canonical.path());
    let policy = WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    };
    let executor = WorkflowExecutor::new(store.clone(), policy.clone());
    let run = executor
        .start(impl_fanout_spec(canonical.path(), targets))
        .unwrap();
    let cfg = WriteCoordinatorConfig::default();
    let runtime = resolve_write_coordinator_runtime(canonical.path(), &cfg);
    let run_root = canonical.path().join(".archon/workflows").join(&run.id);
    let ctx = ctx_for(&store, &run, &policy, run_root);
    let runner = WritingRunner {
        content: "// w\n".into(),
        record_inputs: std::sync::Mutex::new(vec![]),
    };
    let outcome = block_on(run_coordinated_implementation_fanout(
        &ctx,
        plan_inputs(targets),
        &runtime,
        &cfg,
        &runner,
    ))
    .expect("coordinated fanout");
    assert_eq!(outcome.waves.len(), 2, "overlapping targets must serialize");
}

#[test]
fn boundary_unavailable_forces_serial_fallback() {
    let canonical = canonical_repo();
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"])];
    let store = WorkflowStore::project(canonical.path());
    let policy = WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    };
    let executor = WorkflowExecutor::new(store.clone(), policy.clone());
    let run = executor
        .start(impl_fanout_spec(canonical.path(), targets))
        .unwrap();
    let cfg = WriteCoordinatorConfig::default();
    let runtime = resolve_write_coordinator_runtime(canonical.path(), &cfg);
    let run_root = canonical.path().join(".archon/workflows").join(&run.id);
    let ctx = ctx_for(&store, &run, &policy, run_root);
    let runner = NoBoundaryRunner;
    let outcome = block_on(run_coordinated_implementation_fanout(
        &ctx,
        plan_inputs(targets),
        &runtime,
        &cfg,
        &runner,
    ))
    .expect("returns fallback signal");
    assert_eq!(
        outcome.serial_fallback,
        Some(SerialFallbackReason::BoundaryUnavailable)
    );
    assert!(outcome.waves.is_empty());
}

#[test]
fn canonical_mutation_fails_wave() {
    let canonical = canonical_repo();
    let targets: &[(&str, &[&str])] = &[("a", &["src/a.rs"])];
    let store = WorkflowStore::project(canonical.path());
    let policy = WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::default()
    };
    let executor = WorkflowExecutor::new(store.clone(), policy.clone());
    let run = executor
        .start(impl_fanout_spec(canonical.path(), targets))
        .unwrap();
    let cfg = WriteCoordinatorConfig::default();
    let runtime = resolve_write_coordinator_runtime(canonical.path(), &cfg);
    let run_root = canonical.path().join(".archon/workflows").join(&run.id);
    let ctx = ctx_for(&store, &run, &policy, run_root);
    let runner = CanonicalMutatingRunner {
        canonical: canonical.path().to_path_buf(),
    };
    let outcome = block_on(run_coordinated_implementation_fanout(
        &ctx,
        plan_inputs(targets),
        &runtime,
        &cfg,
        &runner,
    ))
    .expect("coordinated fanout");
    assert_eq!(outcome.waves.len(), 1);
    assert!(
        outcome.waves[0]
            .failure
            .as_deref()
            .is_some_and(|f| f.contains("CanonicalMutation")),
        "wave must fail with canonical mutation, got {:?}",
        outcome.waves[0].failure
    );
    // The tampered file is a declared target; no patch from the failed wave applied.
    assert_ne!(
        outcome.item_status.get("implement-0"),
        Some(&ManifestStatus::Applied)
    );
}
