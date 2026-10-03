//! Missing memory always refuses; sidecars never contribute authority.
use super::*;

#[test]
fn unknown_agents_refuse_with_the_exact_recovery_even_without_history() {
    let temp = tempfile::tempdir().unwrap();
    let store = AgentTranscriptStore::with_base_dir(temp.path().into());
    let manager = SubagentManager::new(4);
    let error = plan_resume(&store, &manager, "unknown", "next").unwrap_err();
    assert_eq!(
        error,
        "cannot resume agent 'unknown': its confinement is only known to the process that started it; start a new agent"
    );
}

#[test]
fn forged_and_malformed_metadata_cannot_claim_unconfined() {
    let temp = tempfile::tempdir().unwrap();
    let store = AgentTranscriptStore::with_base_dir(temp.path().into());
    let manager = SubagentManager::new(4);
    store.record_message(
        "child",
        &serde_json::json!({"role":"assistant","content":"done"}),
    );
    for metadata in [
        r#"{"agent_type":"general-purpose","confinement":{"isolation":"unset"}}"#,
        "garbage",
    ] {
        std::fs::write(store.metadata_path("child"), metadata).unwrap();
        let error = plan_resume(&store, &manager, "child", "next").unwrap_err();
        assert_eq!(error, unknown_context("child"));
    }
}

#[test]
fn sidecar_retains_only_the_original_descriptive_fields() {
    let metadata = super::super::AgentMetadata {
        agent_type: "worker".into(),
        worktree_path: Some("/work".into()),
        description: Some("task".into()),
        filename: Some("worker.md".into()),
    };
    let json = serde_json::to_value(metadata).unwrap();
    assert_eq!(json.as_object().unwrap().len(), 4);
    assert!(json.get("confinement").is_none());
    let legacy = serde_json::json!({"agent_type":"worker","confinement":"invalid"});
    let parsed: super::super::AgentMetadata = serde_json::from_value(legacy).unwrap();
    assert_eq!(parsed.agent_type, "worker");
}

fn pending(agent_id: &str) -> PendingResume {
    ResumePlan {
        request: SubagentRequest {
            prompt: String::new(),
            model: None,
            allowed_tools: vec![],
            max_turns: 1,
            timeout_secs: 1,
            subagent_type: None,
            run_in_background: false,
            cwd: None,
            isolation: None,
            read_roots: vec![],
            write_roots: vec![],
            provider_env: None,
        },
        messages: vec![],
        agent_id: agent_id.into(),
        generation: 1,
    }
    .into_pending()
    .1
}

#[tokio::test]
async fn a_carried_resume_reaches_only_its_own_agents_run() {
    assert!(PendingResume::carried("a").is_none());
    pending("a")
        .carry(async {
            assert_eq!(PendingResume::carried("a").unwrap().generation, 1);
            assert!(
                PendingResume::carried("b").is_none(),
                "another id's run took the resume"
            );
            // A run of the same id started elsewhere (a reused id) cannot take it.
            let elsewhere = tokio::spawn(async { PendingResume::carried("a").is_none() });
            assert!(elsewhere.await.unwrap(), "another run of the id took the resume");
        })
        .await;
    assert!(PendingResume::carried("a").is_none());
}
