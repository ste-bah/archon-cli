use super::*;

#[test]
fn slash_workflow_decompose_parses_host_owned_paths() {
    let request = crate::command::fixed_decomposition_host::parse_slash_args(&[
        "decompose".into(),
        "--prd".into(),
        "prds/PRD-X-001.md".into(),
        "--tasks".into(),
        "tasks/PRD-X-001".into(),
    ])
    .unwrap();
    assert_eq!(
        request.prd_path,
        std::path::PathBuf::from("prds/PRD-X-001.md")
    );
    assert_eq!(
        request.task_root,
        std::path::PathBuf::from("tasks/PRD-X-001")
    );
}

#[test]
fn slash_workflow_decompose_gate_off_refuses_before_run_creation() {
    let temp = tempfile::tempdir().unwrap();
    let (mut ctx, _rx) = CtxBuilder::new()
        .with_working_dir(temp.path().to_path_buf())
        .build();
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.gate_mode = archon_core::config::GateMode::Off;
    ctx.workflow_config = Some(config);
    ctx.workflow_env_vars = Some(archon_core::env_vars::load_env_vars_from(
        &std::collections::HashMap::new(),
    ));

    let error = WorkflowHandler
        .execute(
            &mut ctx,
            &[
                "decompose".into(),
                "--prd".into(),
                "prds/PRD-X-001.md".into(),
                "--tasks".into(),
                "tasks/PRD-X-001".into(),
            ],
        )
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        crate::command::workflow_decompose::DECOMPOSE_GATE_OFF_REMEDY
    );
    assert!(!temp.path().join(".archon/workflows").exists());
}

#[test]
fn workflow_prd_spec_source_delegates_without_agent_or_shell_relay() {
    let source = include_str!("../../crates/archon-core/src/skills/workflow_prd_spec.rs");
    let production = source.split("#[cfg(test)]").next().unwrap();
    assert!(production.contains("SkillOutput::WorkflowDecompose"));
    assert!(!production.contains("SkillOutput::Prompt"));
    assert!(!production.contains("TaskCreate"));
    assert!(!production.contains("Command::new"));
}

#[test]
fn slash_fixed_resume_is_claimed_before_generic_live_dispatch() {
    let source = include_str!("fixed_decomposition_host.rs")
        .split("#[cfg(test)]")
        .next()
        .expect("production source");
    assert!(source.contains("is_fixed_decomposition_run"));
    assert!(source.contains("spawn_resume"));
    assert!(source.contains("resume_fixed_decomposition_with_factory_and_sink"));
}

#[test]
fn fixed_resume_has_no_continue_alias() {
    let parsed = crate::command::fixed_decomposition_host::parse_resume_args(&[
        "continue".into(),
        "wf-fixed".into(),
    ])
    .unwrap();
    assert_eq!(parsed, None);
}

#[test]
fn fixed_resume_parser_accepts_canonical_tui_forms() {
    assert_eq!(
        crate::command::fixed_decomposition_host::parse_resume_args(&[
            "resume".into(),
            "--live".into(),
            "wf-fixed".into(),
        ])
        .unwrap()
        .as_deref(),
        Some("wf-fixed")
    );
    assert_eq!(
        crate::command::fixed_decomposition_host::parse_resume_args(&[
            "resume".into(),
            "wf-fixed".into(),
        ])
        .unwrap()
        .as_deref(),
        Some("wf-fixed")
    );
}

#[test]
fn slash_fixed_resume_routes_to_fixed_gate_before_generic_live_spawn() {
    let project = tempfile::tempdir().unwrap();
    let run_id = "wf-fixed-route";
    let store = archon_workflow::WorkflowStore::project(project.path().canonicalize().unwrap());
    std::fs::create_dir_all(store.run_dir(run_id)).unwrap();
    store
        .write_run_json(
            run_id,
            crate::command::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH,
            &archon_workflow::FixedDecompositionStateV1 {
                schema_version: archon_workflow::FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
                run_kind: archon_workflow::WorkflowRunKind::FixedDecompositionV1,
                identity: archon_workflow::FixedRunIdentityV1 {
                    template_version: archon_workflow::FIXED_DECOMPOSITION_TEMPLATE_VERSION.into(),
                    starting_binary_revision: "rev".into(),
                    script_digest: "script".into(),
                    catalog_digest: "catalog".into(),
                    project_root_identity: project
                        .path()
                        .canonicalize()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    prd_identity: project.path().join("PRD.md").to_string_lossy().into_owned(),
                    task_root_identity: project
                        .path()
                        .join("tasks/PRD-X")
                        .to_string_lossy()
                        .into_owned(),
                },
                phase: archon_workflow::DecompositionPhase::Identity,
                attempts: Default::default(),
                dispositions: Default::default(),
                log_path: project
                    .path()
                    .join("tasks/PRD-X/.decompose.log")
                    .to_string_lossy()
                    .into_owned(),
            },
        )
        .unwrap();
    let (mut ctx, _rx) = CtxBuilder::new()
        .with_working_dir(project.path().to_path_buf())
        .build();
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.gate_mode = archon_core::config::GateMode::Off;
    ctx.workflow_config = Some(config);
    ctx.workflow_env_vars = Some(archon_core::env_vars::load_env_vars_from(
        &std::collections::HashMap::new(),
    ));

    let error = WorkflowHandler
        .execute(&mut ctx, &["resume".into(), "--live".into(), run_id.into()])
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        crate::command::workflow_decompose::DECOMPOSE_GATE_OFF_REMEDY
    );
}
