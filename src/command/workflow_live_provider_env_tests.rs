use super::*;

#[test]
fn provider_env_policy_reads_generic_requirement_aliases() {
    let input = serde_json::json!({
        "item": {
            "required_env_keys": ["POLYGON_API_KEY", "tiingo_token"]
        },
        "provider_env_requirements": ["FMP_API_KEY"]
    });

    let policy = provider_env_policy_from_input(&input).expect("policy");

    assert_eq!(
        policy.required_keys,
        vec![
            "FMP_API_KEY".to_string(),
            "POLYGON_API_KEY".to_string(),
            "TIINGO_TOKEN".to_string()
        ]
    );
    assert_eq!(policy.profile_sources, vec!["~/.profile".to_string()]);
}

#[test]
fn provider_env_policy_reads_declared_keys_from_requirement_object() {
    let input = serde_json::json!({
        "item": {
            "provider_env_requirements": {
                "credentials_required": "conditional",
                "provider": "OpenBB/Polygon",
                "required_keys": ["POLYGON_API_KEY"],
                "requirements": [
                    "Inspect repository docs/scripts for actual credential keys.",
                    "Check only key presence without printing values."
                ]
            }
        }
    });

    let policy = provider_env_policy_from_input(&input).expect("policy");

    assert_eq!(policy.required_keys, vec!["POLYGON_API_KEY".to_string()]);
    assert_eq!(policy.profile_sources, vec!["~/.profile".to_string()]);
}

#[test]
fn undeclared_task_id_does_not_infer_provider_keys() {
    let input = serde_json::json!({
        "item": {
            "canonical_task_ids": ["TASK-TDL-080"],
            "item_id": "impl-TASK-TDL-080-coverage"
        }
    });

    assert!(provider_env_policy_from_input(&input).is_none());
}

#[test]
fn provider_env_result_stamp_persists_redacted_proof_only() {
    let prepared = PreparedProviderEnv {
        policy: ProviderEnvPolicy {
            required_keys: vec!["POLYGON_API_KEY".to_string()],
            profile_sources: vec!["~/.profile".to_string()],
            reason: None,
        },
        proof: ProviderEnvProof {
            profile_sources_checked: vec!["~/.profile".to_string()],
            profile_provenance: Vec::new(),
            resolver: Default::default(),
            redacted_env_keys_checked: vec![archon_tools::provider_env::ProviderEnvKeyProof {
                key: "POLYGON_API_KEY".to_string(),
                state: archon_tools::provider_env::ProviderEnvKeyState::Present,
                found_in: archon_tools::provider_env::ProviderEnvFoundIn::ProcessEnv,
            }],
            credential_state: archon_tools::provider_env::ProviderEnvCredentialState::Present,
            errors: Vec::new(),
        },
    };
    let mut result = archon_workflow::WorkflowV2Result::accepted("ok");

    stamp_provider_env_result(&mut result, Some(&prepared));

    assert_eq!(
        result.data["provider_env_proof"]["credential_state"],
        serde_json::json!("present")
    );
    assert_eq!(
        result.data["provider_env_proof"]["resolver"]["status"],
        serde_json::json!("not_needed")
    );
    assert!(!result.data.to_string().contains("secret"));
}

#[tokio::test]
async fn d47_generated_provider_workflow_resolves_one_shared_key_set() {
    let universe = WorkflowV2TaskUniverse {
        schema_version: "workflow-v2-task-universe-v1".to_string(),
        source_roots: vec!["tasks".to_string()],
        tasks: vec![
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-ALPHA-011".to_string(),
                source_path: "tasks/TASK-ALPHA-011.md".to_string(),
                required_env_keys: vec!["POLYGON_API_KEY".to_string()],
                ..Default::default()
            },
            super::super::super::workflow_live_task_universe::WorkflowV2TaskUniverseTask {
                canonical_task_id: "TASK-ALPHA-019".to_string(),
                source_path: "tasks/TASK-ALPHA-019.md".to_string(),
                required_env_keys: vec!["OPENBB_API_URL".to_string()],
                ..Default::default()
            },
        ],
    };
    let resolved = resolve_generated_workflow_provider_env(Some(&universe))
        .await
        .expect("provider workflow resolution");
    let keys = resolved
        .proof
        .redacted_env_keys_checked
        .iter()
        .map(|proof| proof.key.as_str())
        .collect::<Vec<_>>();

    assert_eq!(keys, vec!["OPENBB_API_URL", "POLYGON_API_KEY"]);
    assert!(!format!("{resolved:?}").contains("https://"));
}
