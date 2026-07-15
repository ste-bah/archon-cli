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
fn provider_env_policy_derives_key_from_provider_requirement_object() {
    let input = serde_json::json!({
        "item": {
            "provider_env_requirements": {
                "credentials_required": "conditional",
                "provider": "OpenBB/Polygon",
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
fn d40_tdl_080_always_gets_profile_sourced_polygon_preflight() {
    let input = serde_json::json!({
        "item": {
            "canonical_task_ids": ["TASK-TDL-080"],
            "item_id": "impl-TASK-TDL-080-coverage"
        }
    });

    let policy = provider_env_policy_from_input(&input).expect("TDL-080 policy");

    assert_eq!(policy.required_keys, vec!["POLYGON_API_KEY"]);
    assert_eq!(policy.profile_sources, vec!["~/.profile"]);
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
            redacted_env_keys_checked: vec![archon_tools::provider_env::ProviderEnvKeyProof {
                key: "POLYGON_API_KEY".to_string(),
                state: archon_tools::provider_env::ProviderEnvKeyState::Present,
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
    assert!(!result.data.to_string().contains("secret"));
}

#[tokio::test]
async fn d47_generated_provider_workflow_resolves_one_shared_key_set() {
    let resolved = resolve_generated_workflow_provider_env([
        "TASK-TDL-050".to_string(),
        "TASK-TDL-080".to_string(),
    ])
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
