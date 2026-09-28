//! Batch G: every agent call runs under the project-input tripwire, and a
//! read-only call's boundary seals the host's roots.
use super::*;
use archon_workflow::WorkflowV2CallExecution;

struct Run {
    _dir: tempfile::TempDir,
    project: PathBuf,
    repo: PathBuf,
    scratch: PathBuf,
    store: WorkflowV2ResultStore,
    spec: PathBuf,
}

fn run() -> Run {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let project = base.join("project");
    let repo = base.join("repo");
    let scratch = base.join("observations");
    let run_root = project.join(".archon/workflows/wf-test");
    let spec = project.join(".archon/lab/strategies/s1/strategy-spec.json");
    for path in [
        run_root.join("v2"),
        project.join("tasks"),
        repo.clone(),
        spec.parent().unwrap().to_path_buf(),
    ] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(&spec, "{\"datasets\":[\"a\"]}").unwrap();
    let policy = serde_json::json!({
        "repository": repo, "project": project, "task_root": project.join("tasks"),
        "scratch_parent": scratch, "project_inputs": [".archon/lab"], "combined": true,
        "toolchain_path": "/usr/bin:/bin", "environment": {}, "cargo_seed": null,
        "timeout_secs": 10, "output_bytes": 1024, "scratch_bytes": 1u64 << 30,
    });
    std::fs::write(
        run_root.join("v2/generated-metadata.json"),
        serde_json::to_vec(
            &serde_json::json!({"observer_snapshot": {"native_execution": {"policy": policy}}}),
        )
        .unwrap(),
    )
    .unwrap();
    Run {
        _dir: dir,
        project,
        repo,
        scratch,
        store: WorkflowV2ResultStore::new(run_root.join("v2")),
        spec,
    }
}

/// The live incident: a read-only verifier's call rewrote a tracked project
/// input and returned an accepted verdict. The verdict is replaced by a
/// FAILED result with a HIGH gap naming the call, and the input is restored.
#[tokio::test]
async fn an_agent_call_that_changes_a_project_input_fails_and_is_restored() {
    let run = run();
    let spec = run.spec.clone();
    let call_id = "verification-wave-review-verify-task-trading-012-1-88";
    let result = with_input_tripwire(Some(&run.store), call_id, &[], async move {
        std::fs::write(&spec, "{\"datasets\":[\"a\",\"regenerated\"]}").unwrap();
        Ok(WorkflowV2Result::accepted("regenerated the stale spec"))
    })
    .await
    .unwrap();
    assert_eq!(result.status, WorkflowV2Status::Failed);
    let gap = &result.residual_gaps[0];
    assert_eq!(gap.severity.as_deref(), Some("high"));
    assert!(gap.id.contains("012-1-88"), "{}", gap.id);
    assert!(
        gap.description.contains("ENVIRONMENT VIOLATION"),
        "{}",
        gap.description
    );
    assert!(result.data["environment_violation"]["changed"][0]["restored"] == true);
    assert_eq!(
        std::fs::read_to_string(&run.spec).unwrap(),
        "{\"datasets\":[\"a\"]}"
    );

    // A call that changes nothing keeps its own verdict.
    let quiet = with_input_tripwire(Some(&run.store), "quiet", &[], async {
        Ok(WorkflowV2Result::accepted("verified"))
    })
    .await
    .unwrap();
    assert_eq!(quiet.status, WorkflowV2Status::Accepted);

    // A pause still unwinds as a pause, though the input is restored.
    let spec = run.spec.clone();
    let paused = with_input_tripwire(Some(&run.store), "paused", &[], async move {
        std::fs::write(&spec, "half-written").unwrap();
        Err(WorkflowError::ControlPaused("run control".into()))
    })
    .await;
    assert!(
        matches!(paused, Err(WorkflowError::ControlPaused(_))),
        "{paused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&run.spec).unwrap(),
        "{\"datasets\":[\"a\"]}"
    );
}

/// Review H1: a write branch's declared project delivery, stamped writable
/// by the write layer and judged where it is, is its work, not a violation;
/// the delivery is recorded so the divergence repair leaves it alone. Its
/// read-only verifier gets no such exemption.
#[tokio::test]
async fn a_write_calls_stamped_delivery_is_its_work() {
    let run = run();
    let mut execution = WorkflowV2CallExecution {
        call: archon_workflow::WorkflowV2HostCall {
            id: "review-remediate-task-x-1-1".into(),
            method: archon_workflow::WorkflowV2HostMethod::Implementation,
            write_mode: Some(archon_workflow::WorkflowV2WriteMode::Worktree),
            options: Default::default(),
        },
        input: serde_json::json!({
            archon_workflow::agent_dispatch_port::WRITE_BOUNDARY_INPUT_KEY:
                {"sealed": [run.project], "writable": [run.spec]},
        }),
        depends_on: Vec::new(),
    };
    let worktree = run
        .store
        .root()
        .join("worktrees/x/x-0")
        .display()
        .to_string();
    let own = own_work(&execution, Some(&worktree));
    assert!(own.contains(&run.spec), "{own:?}");
    let spec = run.spec.clone();
    let delivered = with_input_tripwire(Some(&run.store), &execution.call.id, &own, async move {
        std::fs::write(&spec, "{\"datasets\":[\"delivered\"]}").unwrap();
        Ok(WorkflowV2Result::accepted("delivered"))
    })
    .await
    .unwrap();
    assert_eq!(delivered.status, WorkflowV2Status::Accepted);
    let ledger = std::fs::read_to_string(
        run.store
            .run_root()
            .join("write-coordination/project-inputs-delivered.jsonl"),
    )
    .unwrap();
    assert!(ledger.contains("strategy-spec.json"), "{ledger}");

    execution.call.write_mode = None;
    assert!(own_work(&execution, Some(&worktree)).is_empty());
}

/// A read-only call's boundary seals its working root, the project, the
/// checkout, the acceptance policy's roots and the transcript store, and
/// names nothing writable.
#[test]
fn a_read_only_boundary_seals_the_hosts_roots() {
    let run = run();
    let checkout = run.repo.display().to_string();
    let project = run.project.display().to_string();
    let scope = read_only_boundary(
        Some(&run.store),
        Some(&checkout),
        Some(&project),
        Some(&checkout),
    )
    .expect("a boundary");
    let guard = archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(
        &archon_tools::workflow_read_guard::WorkflowReadGuardSettings::default(),
    )
    .with_read_only_boundary(scope)
    .with_run_store(super::super::run_store_scope(
        Some(&run.store),
        Some(&checkout),
        None,
    ));
    let paths = guard.boundary_paths().expect("bounded");
    for sealed in [
        run.spec.clone(),
        run.repo.join("src/lib.rs"),
        run.scratch.join("evidence/x.json"),
        run.store.run_root().join("state.json"),
        run.store.run_root().join("artifacts/report.json"),
    ] {
        assert!(
            paths.refuses(&sealed),
            "{} must be sealed",
            sealed.display()
        );
    }
    if let Some(home) = dirs::home_dir() {
        assert!(paths.refuses(&home.join(".archon/sessions/wf-x/subagents/t.jsonl")));
    }
    // Only the roots' toolchain directories are writable.
    assert!(!paths.refuses(&run.repo.join("target/debug/x")));
    assert!(!paths.refuses(&run.repo.join(".pytest_cache/v/x")));
    assert!(
        paths.writable.iter().all(|dir| [&run.repo, &run.project]
            .iter()
            .any(|root| dir.starts_with(root) && dir != *root)),
        "{:?}",
        paths.writable
    );
    assert!(read_only_boundary(Some(&run.store), None, Some(&project), None).is_none());
}
