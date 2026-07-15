use super::*;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/d36_raw_artifact_overreach_noop_loop.json"
    ))
    .expect("D36 fixture")
}

#[test]
fn d36_routes_unplanned_raw_task_identity_checks_to_corrected_retries() {
    let fixture = fixture();
    let triage = reroute_unplanned_raw_task_identity(
        fixture["triage"].clone(),
        fixture["verification_plan"].as_array().expect("plan"),
    );
    let data = &triage["result"]["data"];

    assert!(support::array(data.get("implementation_failures")).is_empty());
    assert_eq!(support::array(data.get("retry_items")).len(), 3);
    assert_eq!(support::array(data.get("overreach_corrections")).len(), 3);
    assert!(support::array(data.get("retry_items")).iter().all(|item| {
        item["classification"] == "retryable_verification_shape_issue"
            && item["verification_failure_class"] == "artifact_contract_overreach"
    }));
}

#[test]
fn d36_corrected_retry_preserves_real_tdl050_policy_check() {
    let fixture = fixture();
    let triage = reroute_unplanned_raw_task_identity(
        fixture["triage"].clone(),
        fixture["verification_plan"].as_array().expect("plan"),
    );
    let retry = support::array(triage.pointer("/result/data/retry_items"))
        .into_iter()
        .find(|item| support::strings_of(item.get("canonical_task_ids")) == ["TASK-TDL-050"])
        .expect("TDL-050 retry");
    let expected = searchable_text(retry.get("expected_evidence").unwrap_or(&Value::Null));

    assert!(expected.contains("adjusted/unadjusted policy"));
    assert!(!demands_task_identity(&expected));
}

#[test]
fn d36_fixture_raw_provider_payloads_are_not_task_tagged() {
    let fixture = fixture();
    for artifact in support::array(fixture.get("raw_provider_artifacts")) {
        let content = searchable_text(artifact.get("content").unwrap_or(&Value::Null));
        assert!(!content.contains("task_id"));
        assert!(!content.contains("task-tdl-"));
    }
}

#[test]
fn explicit_task_id_artifact_contract_remains_actionable() {
    let fixture = fixture();
    let mut plans = fixture["verification_plan"]
        .as_array()
        .expect("plan")
        .clone();
    plans[0]["expected_evidence"] =
        Value::String("raw/request.json must contain task_id TASK-TDL-040".to_string());
    let triage = reroute_unplanned_raw_task_identity(fixture["triage"].clone(), &plans);

    assert_eq!(
        support::array(triage.pointer("/result/data/implementation_failures")).len(),
        1
    );
    assert_eq!(
        support::array(triage.pointer("/result/data/retry_items")).len(),
        2
    );
}

#[test]
fn d36_prompts_ground_artifact_fields_in_task_contracts() {
    let prompts = [
        super::super::workflow_live_v2_lifecycle_prompts::VERIFICATION_PLAN_TASK,
        super::super::workflow_live_v2_lifecycle_prompts::VERIFICATION_FAILURE_TRIAGE_TASK,
    ];
    for prompt in prompts {
        assert!(prompt.to_ascii_lowercase().contains("task"));
        assert!(prompt.contains("raw/request.json"));
        assert!(prompt.contains("canonical task ID"));
    }
}

#[test]
fn d54_routes_host_manifest_schema_overreach_to_grounded_retry() {
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/wf44_host_manifest_schema_overreach.json"
    ))
    .expect("D54 fixture");

    let triage = reroute_unplanned_raw_task_identity(
        fixture["triage"].clone(),
        fixture["verification_plan"].as_array().expect("plan"),
    );
    let data = &triage["data"];
    let retries = support::array(data.get("retry_items"));

    assert!(support::array(data.get("implementation_failures")).is_empty());
    assert_eq!(retries.len(), 1);
    assert_eq!(
        retries[0]["verification_failure_class"],
        "host_manifest_schema_overreach"
    );
    let retry_text = searchable_text(&retries[0]);
    assert!(retry_text.contains("patch_manifest.v1"));
    assert!(retry_text.contains("run-scoped"));
}

#[test]
fn d54_verification_prompts_whitelist_the_host_manifest_schema() {
    let prompts = [
        super::super::workflow_live_v2_lifecycle_prompts::VERIFICATION_PLAN_TASK,
        super::super::workflow_live_v2_lifecycle_prompts::VERIFICATION_WAVE_TASK,
        super::super::workflow_live_v2_lifecycle_prompts::POST_REMEDIATION_VERIFICATION_PLAN_TASK,
        super::super::workflow_live_v2_lifecycle_prompts::POST_REMEDIATION_VERIFICATION_WAVE_TASK,
        super::super::workflow_live_v2_lifecycle_prompts::RETRY_VERIFICATION_WAVE_TASK,
    ];
    for prompt in prompts {
        let grounded =
            super::super::workflow_live_v2_lifecycle_prompts::ground_host_manifest_schema(prompt);
        assert!(grounded.contains("archon.workflow.patch_manifest.v1"));
        assert!(grounded.contains("provider_env_proof is run-scoped"));
        assert!(grounded.contains("normalized_path"));
    }
}
