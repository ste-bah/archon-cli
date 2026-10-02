//! Batch G: every agent call runs under the project-input tripwire, and a
//! read-only call's boundary seals the host's roots. Batch G2: a violation
//! is resolved by the host (re-run, operational error), a write branch seals
//! the same roots, and a call with no working root is bounded too.
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
        "toolchain_path": std::env::join_paths([base.join("tools")]).unwrap().into_string().unwrap(),
        "environment": {}, "cargo_seed": null,
        "timeout_secs": 10, "output_bytes": 1024, "scratch_bytes": 1u64 << 30,
    });
    serde_json::from_value::<archon_workflow::acceptance_scratch::ScratchPolicy>(policy.clone())
        .unwrap()
        .validate()
        .expect("the recorded tripwire policy must be valid on this host");
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

fn counting<'a>(
    spec: &'a Path,
    writes_on: &'a [usize],
    attempts: &'a std::sync::atomic::AtomicUsize,
) -> impl FnMut() -> std::pin::Pin<
    Box<dyn std::future::Future<Output = WorkflowResult<WorkflowV2Result>> + Send + 'a>,
> + 'a {
    move || {
        Box::pin(async move {
            let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if writes_on.contains(&attempt) {
                std::fs::write(spec, format!("{{\"datasets\":[\"a\",\"{attempt}\"]}}")).unwrap();
            }
            Ok(WorkflowV2Result::accepted("regenerated the stale spec"))
        })
    }
}

/// The live incident: a read-only verifier's call rewrote a tracked project
/// input and returned an accepted verdict. Batch G2: the host puts the input
/// back and re-runs the call once; the re-run's verdict stands, and neither
/// the task nor its branch is charged (no failed result, no gap).
#[tokio::test]
async fn a_call_that_changes_a_project_input_is_restored_and_re_run_once() {
    let run = run();
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    let call_id = "verification-wave-review-verify-task-x-1-88";
    let result = with_input_tripwire(
        Some(&run.store),
        call_id,
        &[],
        counting(&run.spec, &[0], &attempts),
    )
    .await
    .unwrap();
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(result.status, archon_workflow::WorkflowV2Status::Accepted);
    assert!(
        result.residual_gaps.is_empty(),
        "{:?}",
        result.residual_gaps
    );
    assert!(
        result
            .evidence
            .iter()
            .any(|e| e.summary.contains("the host re-ran this call once")),
        "{:?}",
        result.evidence
    );
    assert_eq!(
        std::fs::read_to_string(&run.spec).unwrap(),
        "{\"datasets\":[\"a\"]}"
    );

    // A call that changes nothing keeps its own verdict, run once.
    let quiet = with_input_tripwire(Some(&run.store), "quiet", &[], || async {
        Ok(WorkflowV2Result::accepted("verified"))
    })
    .await
    .unwrap();
    assert_eq!(quiet.status, archon_workflow::WorkflowV2Status::Accepted);

    // A pause still unwinds as a pause, though the input is restored.
    let spec = run.spec.clone();
    let paused = with_input_tripwire(Some(&run.store), "paused", &[], || {
        let spec = spec.clone();
        async move {
            std::fs::write(&spec, "half-written").unwrap();
            Err(WorkflowError::ControlPaused("run control".into()))
        }
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

/// A call whose re-run changes the inputs again is the host's operational
/// error: typed so the scheduler files it with transport failures, never a
/// failed verdict with a gap a task would be sent to fix.
#[tokio::test]
async fn a_call_that_trips_twice_is_a_host_operational_error() {
    let run = run();
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    let error = with_input_tripwire(
        Some(&run.store),
        "verification-wave-x-2",
        &[],
        counting(&run.spec, &[0, 1], &attempts),
    )
    .await
    .expect_err("an operational error");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    let text = error.to_string();
    assert!(
        archon_workflow::error::is_host_operational_text(&text),
        "{text}"
    );
    assert!(text.contains("ENVIRONMENT VIOLATION"), "{text}");
    assert_eq!(
        std::fs::read_to_string(&run.spec).unwrap(),
        "{\"datasets\":[\"a\"]}"
    );
}

/// Batch G2 (G2-3c): a write call's window overlapped by another watched
/// call is not charged for a change it was not attributed: its work is its
/// worktree's, the input is restored, and it runs once. Attributed (nothing
/// overlapped), it is the host's operational error, never re-run in place.
#[tokio::test]
async fn a_write_call_keeps_its_result_for_a_change_it_is_not_attributed() {
    let run = run();
    let worktree = run.store.root().join("worktrees/x/x-0");
    std::fs::create_dir_all(&worktree).unwrap();
    let own = vec![worktree.clone()];
    let sibling = archon_workflow::write_coordinator::input_tripwire::InputTripwire::arm(
        run.store.run_root(),
    )
    .expect("a sibling window");
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    let result = with_input_tripwire(
        Some(&run.store),
        "implementation-x-1",
        &own,
        counting(&run.spec, &[0], &attempts),
    )
    .await
    .unwrap();
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(result.status, archon_workflow::WorkflowV2Status::Accepted);
    assert!(result.residual_gaps.is_empty());
    assert!(result.data["environment_violation_unattributed"]["attributed"] == false);
    assert_eq!(
        std::fs::read_to_string(&run.spec).unwrap(),
        "{\"datasets\":[\"a\"]}"
    );
    // The sibling is told too, and is not attributed it either.
    let told = sibling.check("sibling").expect("the sibling saw it");
    assert!(!told.attributed);

    // Attributed (nothing overlapped): never re-run in its worktree, where
    // the untrusted attempt's edits would land with the re-run's; the
    // host's operational error instead, retried like a dropped transport.
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    let error = with_input_tripwire(
        Some(&run.store),
        "implementation-x-2",
        &own,
        counting(&run.spec, &[0], &attempts),
    )
    .await
    .expect_err("an operational error");
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(
        matches!(error, WorkflowError::HostOperational(_)),
        "{error:?}"
    );
    // The script is handed a refundable, execution-typed failure.
    let handed =
        archon_workflow::v2::host_fault::v2_result_for_call_error("implementation-x-2", &error);
    assert_eq!(
        handed.data["transport_failure_no_verdict"], true,
        "{handed:#?}"
    );
    assert_eq!(handed.data["failure_kind"], "execution");
    // The run's terminal facts read the record's summary: a stage the host
    // could not trust is Blocked (resumable), never a task's Failed.
    assert!(archon_workflow::error::is_host_operational_text(
        &handed.summary
    ));
}

/// Batch G2 (G2-2): a write call's own work is its working tree and what the
/// host stamped writable there; a live project path stamped writable (the
/// pre-G2 declared-artifact grant) is never exempt from the tripwire.
#[test]
fn a_write_calls_own_work_never_exempts_the_live_project_root() {
    let run = run();
    let worktree = run.store.root().join("worktrees/x/x-0");
    let seeded = worktree.join(".archon/lab/strategies/s1/strategy-spec.json");
    let mut execution = WorkflowV2CallExecution {
        call: archon_workflow::WorkflowV2HostCall {
            id: "review-remediate-task-x-1-1".into(),
            method: archon_workflow::WorkflowV2HostMethod::Implementation,
            write_mode: Some(archon_workflow::WorkflowV2WriteMode::Worktree),
            options: Default::default(),
        },
        input: serde_json::json!({
            archon_workflow::agent_dispatch_port::WRITE_BOUNDARY_INPUT_KEY:
                {"sealed": [run.project], "writable": [run.spec, seeded]},
        }),
        depends_on: Vec::new(),
    };
    let own = own_work(&execution, worktree.to_str(), Some(&run.store));
    assert!(own.contains(&worktree), "{own:?}");
    assert!(own.contains(&seeded), "{own:?}");
    assert!(!own.contains(&run.spec), "{own:?}");
    execution.call.write_mode = None;
    assert!(own_work(&execution, worktree.to_str(), Some(&run.store)).is_empty());
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
    // Batch G2 (G2-4): a call with no working root is bounded too, in this
    // process's directory, with every host root sealed.
    let scope = read_only_boundary(Some(&run.store), None, Some(&project), None)
        .expect("always a boundary");
    let paths = archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(
        &archon_tools::workflow_read_guard::WorkflowReadGuardSettings::default(),
    )
    .with_read_only_boundary(scope)
    .boundary_paths()
    .expect("bounded without a working root");
    for sealed in [
        run.spec.clone(),
        run.scratch.join("acceptance-evidence-x/observation.json"),
        run.store.run_root().join("state.json"),
        run.repo.join("src/lib.rs"),
    ] {
        assert!(paths.refuses(&sealed), "{}", sealed.display());
    }
}

/// Batch G2 (G2-1): a write branch's boundary seals the SAME host roots a
/// read-only call's does -- the acceptance evidence under the scratch
/// parent, the transcript store and configuration included -- so a coder's
/// shell cannot rewrite an `observation.json` the host reads as a verdict.
#[tokio::test]
async fn a_write_branch_cannot_write_the_hosts_evidence_or_stores() {
    let run = run();
    let worktree = run.store.root().join("worktrees/x/x-0");
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    let evidence = run
        .scratch
        .join("acceptance-evidence-1234/observation.json");
    std::fs::create_dir_all(evidence.parent().unwrap()).unwrap();
    std::fs::write(&evidence, "{\"verdict\":\"failed\"}").unwrap();
    let sealed: Vec<String> = archon_workflow::write_coordinator::sealed_roots::sealed_host_roots(
        Some(run.store.run_root()),
        Some(&run.project),
        Some(&run.repo),
    )
    .iter()
    .map(|p| p.display().to_string())
    .collect();
    let input = serde_json::json!({
        archon_workflow::agent_dispatch_port::DECLARED_TARGETS_INPUT_KEY: ["src/lib.rs"],
        archon_workflow::agent_dispatch_port::ISOLATED_WORKTREE_INPUT_KEY: true,
        archon_workflow::agent_dispatch_port::WRITE_BOUNDARY_INPUT_KEY:
            {"sealed": sealed, "writable": []},
    });
    let guard = archon_tools::workflow_read_guard::WorkflowReadGuard::from_settings(
        &archon_tools::workflow_read_guard::WorkflowReadGuardSettings::default(),
    )
    .with_declared_targets(super::super::live_agent_dispatch::declared_target_scope(
        &input,
        worktree.to_str(),
    ));
    let paths = guard
        .boundary_paths()
        .expect("an isolated branch is bounded");
    for sealed in [
        evidence.clone(),
        run.spec.clone(),
        run.store.run_root().join("state.json"),
        run.repo.join("src/lib.rs"),
    ] {
        assert!(paths.refuses(&sealed), "{}", sealed.display());
    }
    for store in archon_workflow::write_coordinator::sealed_roots::user_host_stores() {
        assert!(
            paths.refuses(&store.join("x")) || paths.refuses(&store),
            "{}",
            store.display()
        );
    }
    assert!(!paths.refuses(&worktree.join("src/lib.rs")));
    // The file tools are refused by the same sets.
    let refused = guard.before_tool(
        "Write",
        &serde_json::json!({"file_path": evidence, "content": "{\"verdict\":\"passed\"}"}),
    );
    assert!(
        refused.is_some(),
        "a file-tool write to the evidence is refused"
    );
    // And the shell, at the OS level, where the platform can bound it.
    let ctx = archon_tools::tool::ToolContext {
        working_dir: worktree.clone(),
        session_id: "g2-write-boundary".into(),
        workflow_read_guard: Some(std::sync::Arc::new(guard)),
        ..archon_tools::tool::ToolContext::default()
    };
    let result = archon_tools::tool::Tool::execute(
        &archon_tools::bash::BashTool::default(),
        serde_json::json!({"command": format!(
            "printf '{{\"verdict\":\"passed\"}}' > {} ; cat {}",
            evidence.display(),
            evidence.display()
        )}),
        &ctx,
    )
    .await;
    if archon_tools::bash::shell_write_boundary_available() {
        assert_eq!(
            std::fs::read_to_string(&evidence).unwrap(),
            "{\"verdict\":\"failed\"}",
            "{}",
            result.content
        );
    }
}
