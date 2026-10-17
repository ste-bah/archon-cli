use archon_core::agents::AgentRegistry;
use archon_pipeline::{runner::PipelineType, session::SessionStatus};

#[tokio::test]
async fn legacy_resume_preflights_and_reports_every_missing_agent_key() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path();
    let temp_home = tempfile::tempdir().unwrap();
    let registry = AgentRegistry::load_with_user_home(cwd, Some(temp_home.path()));
    let missing: Vec<_> = archon_pipeline::required_builtin_agent_keys(PipelineType::Coding)
        .into_iter()
        .filter(|key| registry.resolve(key).is_none())
        .collect();
    assert!(
        !missing.is_empty(),
        "test project must be missing bundled profiles"
    );
    let mut session = archon_pipeline::session::new_session(PipelineType::Coding, "legacy resume");
    session.status = SessionStatus::Interrupted;
    archon_pipeline::session::checkpoint(&session, cwd).unwrap();
    let checkpoint = cwd
        .join(".pipeline-state")
        .join(&session.session_id)
        .join("checkpoint.json");
    let before = std::fs::read(&checkpoint).unwrap();

    let error = super::handle_legacy_resume_with_user_home(
        cwd,
        &session.session_id,
        &archon_core::config::ArchonConfig::default(),
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        Some(temp_home.path()),
    )
    .await
    .unwrap_err()
    .to_string();
    for key in missing {
        assert!(
            error.contains(&key),
            "missing key {key} must be reported: {error}"
        );
    }
    assert!(error.contains("archon agent sync"));
    assert_eq!(
        std::fs::read(checkpoint).unwrap(),
        before,
        "failed preflight must leave saved state byte-identical"
    );
}
