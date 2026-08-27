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
