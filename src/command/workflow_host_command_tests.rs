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
        prd_path,
        task_root,
        run_staging_root: root.join("run/staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment: Default::default(),
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
        ])
    );
    assert!(!catalog.digest.is_empty());
    assert!(catalog.capabilities.values().all(|cap| !cap.detaches));
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

    let resolved = resolve_host_command(&request, &catalog, &context).unwrap();
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
                .join("task-set-lint/gate-envelope.json")
                .to_str()
                .unwrap(),
        ]
    );
    assert!(resolved.environment.is_empty());
    assert_eq!(resolved.stdin, None);
    assert_eq!(
        resolved.remediation_scopes,
        BTreeSet::from([
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

    let error = resolve_host_command(&request, &catalog, &context).unwrap_err();
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
        resolve_host_command(&lint, &catalog, &context)
            .unwrap_err()
            .to_string()
            .contains("does not accept stdin")
    );

    let freeze = HostCommandRequest::new("freeze-acceptance", Some("opaque bytes".into())).unwrap();
    let resolved = resolve_host_command(&freeze, &catalog, &context).unwrap();
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
    let error = resolve_host_command(&request, &catalog, &context).unwrap_err();
    assert!(error.to_string().contains("symlink"), "{error}");
}
