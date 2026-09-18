use std::collections::BTreeSet;
use std::path::PathBuf;

use archon_workflow::{HostCommandRequest, RemediationScope};

use super::workflow_host_command_catalog::{
    HostCommandResolutionContext, fixed_decomposition_catalog, resolve_host_command,
};

fn context(root: &std::path::Path) -> HostCommandResolutionContext {
    let project_root = root.join("project");
    let task_root = project_root.join("tasks/PRD-X");
    let prd_path = project_root.join("prds/PRD-X/PRD-X.md");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::create_dir_all(prd_path.parent().unwrap()).unwrap();
    std::fs::write(&prd_path, "# PRD\n").unwrap();
    HostCommandResolutionContext {
        program: PathBuf::from("/trusted/archon"),
        project_root,
        prd_digest: archon_workflow::task_set_contract::content_digest(
            &std::fs::read(&prd_path).unwrap(),
        ),
        prd_path,
        task_root,
        run_staging_root: root.join("run/staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment: Default::default(),
        gate_mode: archon_core::config::GateMode::Observe,
    }
}

#[test]
fn fixed_catalog_contains_only_reviewed_symbolic_capabilities() {
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    assert_eq!(
        catalog
            .capabilities
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "freeze-acceptance".to_string(),
            "freeze-skeleton".to_string(),
            "land-task-body".to_string(),
            "requirements-trace".to_string(),
            "task-set-lint".to_string(),
            "verify-frozen-acceptance".to_string(),
            "verify-frozen-skeleton".to_string(),
        ])
    );
    assert!(!catalog.digest.is_empty());
    assert!(catalog.capabilities.values().all(|cap| !cap.detaches));
}

/// The frozen-chain verifications (Issue-46) take no candidate, run no
/// provider, write only their envelope, and resolve to the hidden
/// `verify-frozen-chain` child with the stage baked into the argv.
#[test]
fn verify_frozen_chain_capabilities_publish_nothing_but_their_envelope() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    for (id, stage) in [
        ("verify-frozen-acceptance", "acceptance"),
        ("verify-frozen-skeleton", "skeleton"),
    ] {
        let request = HostCommandRequest::new(id, None).unwrap();
        let resolved = resolve_host_command(&request, &catalog, &context, "call-1").unwrap();
        assert_eq!(
            &resolved.args[..4],
            ["workflow", "verify-frozen-chain", "--stage", stage]
        );
        assert!(resolved.stdin.is_none());
        assert!(resolved.environment.is_empty(), "no provider environment");
        assert_eq!(resolved.declared_write_set.len(), 1);
        assert!(resolved.declared_write_set[0].ends_with("gate-envelope.json"));
        assert_eq!(
            resolved.remediation_scopes,
            BTreeSet::from([RemediationScope::Operational])
        );
        let with_stdin = HostCommandRequest::new(id, Some("x".into())).unwrap();
        assert!(resolve_host_command(&with_stdin, &catalog, &context, "call-1").is_err());
    }
}

#[test]
fn changing_any_authority_bearing_catalog_field_changes_digest() {
    let baseline = fixed_decomposition_catalog("rev-1").unwrap();
    let different_revision = fixed_decomposition_catalog("rev-2").unwrap();
    assert_ne!(baseline.digest, different_revision.digest);

    let mut changed = baseline.clone();
    changed
        .capabilities
        .get_mut("task-set-lint")
        .unwrap()
        .timeout_secs += 1;
    changed.recompute_digest().unwrap();
    assert_ne!(baseline.digest, changed.digest);
}

#[test]
fn host_command_resolution_binds_process_authority_from_catalog() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    let request = HostCommandRequest::new("task-set-lint", None).unwrap();

    let resolved = resolve_host_command(&request, &catalog, &context, "call-1").unwrap();
    assert_eq!(resolved.program, PathBuf::from("/trusted/archon"));
    assert_eq!(resolved.cwd, context.project_root);
    assert_eq!(
        resolved.args,
        [
            "workflow",
            "lint",
            "--tasks",
            context.task_root.to_str().unwrap(),
            "--gate-envelope",
            context
                .run_staging_root
                .join("call-1/gate-envelope.json")
                .to_str()
                .unwrap(),
            "--call-id",
            "call-1",
        ]
    );
    // The set gate runs the obligation fidelity audit, so it carries the
    // freeze provider environment (empty in this context) and may return a
    // `Body` finding naming the task whose own text hollows its claim.
    assert!(resolved.environment.is_empty());
    assert_eq!(resolved.stdin, None);
    assert_eq!(
        resolved.remediation_scopes,
        BTreeSet::from([
            RemediationScope::Body,
            RemediationScope::Skeleton,
            RemediationScope::InheritedPredecessor,
            RemediationScope::PrdInput,
            RemediationScope::Operational,
        ])
    );
}

#[test]
fn host_command_resolution_rejects_unknown_capability_before_spawn() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    let request = HostCommandRequest::new("sh -c whoami", None).unwrap();

    let error = resolve_host_command(&request, &catalog, &context, "call-1").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("undeclared host command capability")
    );
}

#[test]
fn lint_capabilities_reject_stdin_and_freeze_capabilities_bind_exact_stdin() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(temp.path());
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();

    let lint = HostCommandRequest::new("task-set-lint", Some("forbidden".into())).unwrap();
    assert!(
        resolve_host_command(&lint, &catalog, &context, "call-1")
            .unwrap_err()
            .to_string()
            .contains("does not accept stdin")
    );

    let freeze = HostCommandRequest::new("freeze-acceptance", Some("opaque bytes".into())).unwrap();
    let resolved = resolve_host_command(&freeze, &catalog, &context, "call-1").unwrap();
    assert_eq!(resolved.stdin.as_deref(), Some(b"opaque bytes".as_slice()));
    assert!(!resolved.args.iter().any(|arg| arg.contains("opaque bytes")));
}

#[cfg(unix)]
#[test]
fn canonical_root_and_prd_tokens_reject_symlink_descent() {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("PRD.md"), "outside").unwrap();
    let link = context.project_root.join("linked");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    context.prd_path = link.join("PRD.md");

    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    let request = HostCommandRequest::new("freeze-acceptance", Some("candidate".into())).unwrap();
    let error = resolve_host_command(&request, &catalog, &context, "call-1").unwrap_err();
    assert!(error.to_string().contains("symlink"), "{error}");
}

#[cfg(unix)]
mod supervisor {
    use std::collections::{BTreeMap, BTreeSet};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use archon_workflow::RemediationScope;

    use super::super::workflow_host_command_catalog::ResolvedHostCommand;
    use super::super::workflow_host_command_supervisor::{
        HostCommandControl, HostCommandSignal, supervise_process_group,
    };

    fn executable(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn command(program: PathBuf) -> ResolvedHostCommand {
        ResolvedHostCommand {
            command_id: "test-fixture".into(),
            program,
            args: Vec::new(),
            cwd: std::env::temp_dir(),
            environment: BTreeMap::new(),
            stdin: None,
            timeout_secs: 5,
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 1024 * 1024,
            declared_write_set: Vec::new(),
            remediation_scopes: BTreeSet::from([RemediationScope::Operational]),
        }
    }

    #[tokio::test]
    async fn supervisor_waits_for_exit_when_the_child_closes_its_pipes_early() {
        // The drain tasks hold the only supervisor-event senders. A child that
        // closes stdout and stderr before exiting ends both tasks, closing the
        // channel while the process is still alive: ordinary end of output, not
        // a supervision failure.
        let temp = tempfile::tempdir().unwrap();
        let program = executable(temp.path(), "early-eof", "exec 1>&- 2>&-\nsleep 1");
        let (control, _handle) = HostCommandControl::new();
        let output = supervise_process_group(command(program), control)
            .await
            .expect("closing the pipes before exit is not a failure");
        assert_eq!(output.exit_code, Some(0));
    }

    #[tokio::test]
    async fn supervisor_drains_large_stdout_and_stderr_without_deadlock() {
        let temp = tempfile::tempdir().unwrap();
        let program = executable(
            temp.path(),
            "both-streams",
            "i=0; while [ $i -lt 1500 ]; do printf 'stdout-%04d-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' \"$i\"; printf 'stderr-%04d-yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy\\n' \"$i\" >&2; i=$((i+1)); done",
        );
        let (control, _handle) = HostCommandControl::new();
        let output = supervise_process_group(command(program), control)
            .await
            .unwrap();

        assert_eq!(output.exit_code, Some(0));
        assert!(output.stdout.len() > 50_000);
        assert!(output.stderr.len() > 50_000);
        assert_eq!(output.stdout_bytes as usize, output.stdout.len());
        assert_eq!(output.stderr_bytes as usize, output.stderr.len());
    }

    #[tokio::test]
    async fn stdout_overflow_terminates_group_and_prevents_late_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let sentinel = temp.path().join("late");
        let program = executable(
            temp.path(),
            "overflow",
            &format!(
                "(sleep 0.4; printf late > '{}') & while :; do printf '0123456789abcdef'; done",
                sentinel.display()
            ),
        );
        let mut request = command(program);
        request.max_stdout_bytes = 256;
        let (control, _handle) = HostCommandControl::new();
        let error = supervise_process_group(request, control)
            .await
            .expect_err("overflow must fail operationally");

        assert!(
            error.to_string().contains("stdout output exceeded"),
            "{error}"
        );
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(!sentinel.exists(), "descendant survived output overflow");
    }

    #[tokio::test]
    async fn timeout_terminates_background_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let sentinel = temp.path().join("late");
        let program = executable(
            temp.path(),
            "timeout",
            &format!(
                "(sleep 0.4; printf late > '{}') & sleep 30",
                sentinel.display()
            ),
        );
        let mut request = command(program);
        request.timeout_secs = 0;
        let (control, _handle) = HostCommandControl::new();
        let error = supervise_process_group(request, control)
            .await
            .expect_err("timeout must fail operationally");

        assert!(error.to_string().contains("timed out"), "{error}");
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(!sentinel.exists(), "descendant survived timeout");
    }

    #[tokio::test]
    async fn pause_and_cancel_interrupt_and_reap_direct_child() {
        for signal in [HostCommandSignal::Paused, HostCommandSignal::Cancelled] {
            let temp = tempfile::tempdir().unwrap();
            let sentinel = temp.path().join("late");
            let program = executable(
                temp.path(),
                "control",
                &format!("sleep 0.4; printf late > '{}'", sentinel.display()),
            );
            let (control, handle) = HostCommandControl::new();
            let task = tokio::spawn(supervise_process_group(command(program), control));
            tokio::time::sleep(Duration::from_millis(30)).await;
            handle.signal(signal).unwrap();
            let error = task.await.unwrap().expect_err("control must interrupt");
            assert!(error.to_string().contains(signal.as_str()), "{error}");
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(!sentinel.exists(), "child survived {}", signal.as_str());
        }
    }

    #[tokio::test]
    async fn supervisor_clears_environment_and_delivers_exact_stdin() {
        let temp = tempfile::tempdir().unwrap();
        let program = executable(
            temp.path(),
            "stdin-env",
            "printf 'declared=%s\\n' \"${DECLARED:-missing}\"; printf 'ambient=%s\\n' \"${ARCHON_R2A_AMBIENT_SENTINEL:-absent}\"; cat",
        );
        let mut request = command(program);
        request
            .environment
            .insert("DECLARED".into(), "allowed".into());
        request.stdin = Some(b"opaque;$(printf not-executed)".to_vec());
        unsafe { std::env::set_var("ARCHON_R2A_AMBIENT_SENTINEL", "must-not-leak") };
        let (control, _handle) = HostCommandControl::new();
        let output = supervise_process_group(request, control).await.unwrap();
        unsafe { std::env::remove_var("ARCHON_R2A_AMBIENT_SENTINEL") };
        let stdout = String::from_utf8(output.stdout).unwrap();

        assert!(stdout.contains("declared=allowed"));
        assert!(stdout.contains("ambient=absent"));
        assert!(stdout.ends_with("opaque;$(printf not-executed)"));
        assert!(!stdout.contains("must-not-leak"));
    }
}

#[test]
fn every_fixed_catalog_argv_parses_through_the_shipped_cli() {
    use clap::Parser;

    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, "---\ntask_id: TASK-X-010\n---\n").unwrap();
    context.frozen_task_id = Some("TASK-X-010".into());
    context.frozen_task_file = Some(task_file);
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();

    for (command_id, stdin) in [
        ("freeze-acceptance", Some("{}".to_string())),
        ("freeze-skeleton", Some("{}".to_string())),
        ("land-task-body", Some("body".to_string())),
        ("task-set-lint", None),
        ("requirements-trace", None),
    ] {
        let request = HostCommandRequest::new(command_id, stdin).unwrap();
        let resolved = resolve_host_command(&request, &catalog, &context, "call-1").unwrap();
        let mut argv = vec!["archon".to_string()];
        argv.extend(resolved.args);
        crate::cli_args::Cli::try_parse_from(argv).unwrap_or_else(|error| {
            panic!("catalog capability {command_id} does not parse: {error}")
        });
    }
}

#[test]
fn fixed_catalog_uses_authoritative_freeze_filenames() {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    };

    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    let acceptance = &catalog.capabilities["freeze-acceptance"].declared_write_set;
    assert!(
        acceptance
            .iter()
            .any(|path| path.ends_with(ACCEPTANCE_CONTRACT_FILE))
    );
    assert!(
        acceptance
            .iter()
            .any(|path| path.ends_with(ACCEPTANCE_LOCK_FILE))
    );
    assert!(!acceptance.iter().any(|path| path.ends_with(".lock.json")));

    let skeleton = &catalog.capabilities["freeze-skeleton"].declared_write_set;
    assert!(
        skeleton
            .iter()
            .any(|path| path.ends_with(TASK_SKELETON_FILE))
    );
    assert!(
        skeleton
            .iter()
            .any(|path| path.ends_with(TASK_SKELETON_LOCK_FILE))
    );
    assert!(!skeleton.iter().any(|path| path.ends_with(".lock.json")));
}

#[test]
fn fixed_catalog_declared_write_sets_match_child_manifest_shapes() {
    use archon_workflow::task_set_contract::{
        ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    };

    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, "---\ntask_id: TASK-X-010\n---\n").unwrap();
    context.frozen_task_id = Some("TASK-X-010".into());
    context.frozen_task_file = Some(task_file);
    let catalog = fixed_decomposition_catalog("rev-1").unwrap();
    let expected = [
        (
            "freeze-acceptance",
            vec![
                ACCEPTANCE_CONTRACT_FILE,
                ACCEPTANCE_LOCK_FILE,
                "acceptance-pin.json",
                "gate-envelope.json",
            ],
            Some("candidate"),
        ),
        (
            "freeze-skeleton",
            vec![
                TASK_SKELETON_FILE,
                TASK_SKELETON_LOCK_FILE,
                "acceptance-pin.json",
                "gate-envelope.json",
            ],
            Some("candidate"),
        ),
        (
            "land-task-body",
            vec!["TASK-X-010.md", "gate-envelope.json"],
            Some("candidate"),
        ),
        ("task-set-lint", vec!["gate-envelope.json"], None),
        ("requirements-trace", vec!["gate-envelope.json"], None),
    ];

    for (id, names, stdin) in expected {
        let request = HostCommandRequest::new(id, stdin.map(str::to_string)).unwrap();
        let resolved = resolve_host_command(&request, &catalog, &context, "call-1").unwrap();
        let actual = resolved
            .declared_write_set
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual,
            names.into_iter().map(str::to_string).collect(),
            "{id}"
        );
    }
}
