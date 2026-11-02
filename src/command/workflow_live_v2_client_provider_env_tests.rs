use super::*;

#[tokio::test]
async fn d47_one_run_uses_identical_provider_presence_for_subagents_and_final_gate() {
    let profile_dir = tempfile::tempdir().expect("profile dir");
    let profile = profile_dir.path().join("profile");
    std::fs::write(&profile, "export ARCHON_DEMO_PROVIDER_KEY=present-value\n").expect("profile");
    let policy = archon_tools::provider_env::ProviderEnvPolicy {
        required_keys: vec!["ARCHON_DEMO_PROVIDER_KEY".to_string()],
        profile_sources: vec![profile.display().to_string()],
        reason: Some("run-scoped invariant".to_string()),
    };
    let resolution = archon_tools::provider_env::resolve_operator_provider_env(
        &policy,
        &policy.required_keys,
        &policy.profile_sources,
    )
    .await;
    let expected_proof = resolution.proof.clone();
    let recorder = Arc::new(RecordingClient::default());
    let (ui_sink, _tui_rx) = crate::command::tui_workflow_ui_sink::default_workflow_ui_sink();
    let client = LiveV2AgentClient::new(
        recorder.clone(),
        ui_sink,
        Vec::new(),
        "wf-provider-invariant".to_string(),
        Some("/repo".to_string()),
        Some(17),
    )
    .with_provider_env_resolution(Some(resolution));

    let mut implementation = request(
        WorkflowV2HostMethod::Implementation,
        Some(WorkflowV2WriteMode::Coordinated),
    );
    implementation.call.id = "implementation-wave-1".to_string();
    let mut verification = request(WorkflowV2HostMethod::Parallel, None);
    verification.call.id = "verification-wave-1".to_string();
    let mut final_gate = request(WorkflowV2HostMethod::Agent, None);
    final_gate.call.id = "final-zero-gap-audit".to_string();

    for request in [&implementation, &verification, &final_gate] {
        client
            .run_agent_request(request, request.task.clone())
            .await
            .expect("recorded workflow request");
    }

    let recorded = recorder.requests.lock().expect("requests lock");
    assert_eq!(recorded.len(), 3);
    for request in recorded.iter() {
        let proof = &request
            .provider_env
            .as_ref()
            .expect("run-scoped provider resolution")
            .downcast_ref::<archon_tools::provider_env::ProviderEnvResolution>()
            .expect("host provider environment")
            .proof;
        assert_eq!(proof, &expected_proof);
    }
    assert!(!format!("{expected_proof:?}").contains("present-value"));
}
