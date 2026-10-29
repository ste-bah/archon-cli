use super::*;

fn envelope(fixture: &Fixture, stage: &str) -> serde_json::Value {
    let path = fixture.project.join("staging/gate-envelope.json");
    super::super::stage_verify(
        &fixture.project,
        stage,
        &fixture.tasks,
        &fixture.prd,
        Some(&path),
        Some("call-1"),
        false,
    )
    .unwrap();
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn frozen_verification_corruption_is_permanent_with_an_exact_restore_action() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    let path = fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE);
    std::fs::write(&path, b"{}").unwrap();
    let found = envelope(&fixture, "acceptance");
    assert_eq!(found["operational_error"]["kind"], "permanent", "{found}");
    let text = found["operational_error"]["text"].as_str().unwrap();
    assert!(
        text.contains(path.to_str().unwrap()) && text.contains("restore"),
        "{text}"
    );
    std::fs::write(&fixture.prd, [0xff]).unwrap();
    let incompatible = envelope(&fixture, "acceptance");
    assert_eq!(
        incompatible["operational_error"]["kind"], "permanent",
        "{incompatible}"
    );
    let text = incompatible["operational_error"]["text"].as_str().unwrap();
    assert!(
        text.contains(fixture.prd.to_str().unwrap()) && text.contains("restore"),
        "the incompatible input needs its own exact remedy: {text}"
    );
}

#[test]
fn frozen_verification_unavailable_input_is_typed_transient_and_recovers() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    let bytes = std::fs::read(&fixture.prd).unwrap();
    std::fs::remove_file(&fixture.prd).unwrap();
    let found = envelope(&fixture, "acceptance");
    assert_eq!(found["operational_error"]["kind"], "transient", "{found}");
    assert_eq!(found["policy_findings"], serde_json::json!([]));
    std::fs::write(&fixture.prd, bytes).unwrap();
    assert!(envelope(&fixture, "acceptance")["operational_error"].is_null());
}

#[test]
fn frozen_verification_authenticated_authored_defects_name_the_owner_and_artifact() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    let path = fixture.tasks.join(TASK_SKELETON_FILE);
    let mut document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    document["tasks"][0]["file_name"] = serde_json::json!("../TASK-X-010.md");
    let bytes = serde_json::to_vec_pretty(&document).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(&fixture.project, &fixture.tasks);
    for (path, field) in [
        (fixture.tasks.join(TASK_SKELETON_LOCK_FILE), "digest"),
        (pin_path, "skeleton_digest"),
    ] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value[field] = digest.clone().into();
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    let found = envelope(&fixture, "skeleton");
    assert!(found["operational_error"].is_null(), "{found}");
    let finding = &found["policy_findings"][0];
    assert_eq!(finding["remediation_scope"], "skeleton", "{found}");
    assert_eq!(finding["subject"], "TASK-X-010", "{found}");
    assert_eq!(
        finding["source_path"],
        path.to_string_lossy().replace('\\', "/"),
        "{found}"
    );
    let fault = serde_json::json!({"gateEnvelope": found}).to_string();
    let driver = crate::command::workflow_decompose::repair_tests::frozen_routing::script(
        "skeleton", &fault,
    )
    .replace(
        "/p/tasks",
        &fixture.tasks.to_string_lossy().replace('\\', "/"),
    );
    let out = crate::command::workflow_decompose::repair_tests::set_gate_skeleton::run(&driver);
    assert!(out.get("error").is_none(), "{out}");
    assert_eq!(
        out["agentCalls"],
        serde_json::json!(["skeleton-author-1", "body-TASK-X-010-author-1"]),
        "{out}"
    );
    document["tasks"][0]["file_name"] = "TASK-X-010.md".into();
    let repaired = crate::command::workflow_task_set::prepare_skeleton_freeze_from_candidate(
        &fixture.project,
        &fixture.tasks,
        &fixture.prd,
        archon_core::config::GateMode::Observe,
        serde_json::to_vec(&document).unwrap(),
    );
    assert!(
        repaired.is_ok(),
        "the real owner freeze must accept the repaired field: {repaired:?}"
    );
}

#[test]
fn frozen_verification_restores_only_digest_authenticated_cached_skeleton_bytes() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    freeze_skeleton(&fixture);
    let before = snapshot(&fixture).unwrap();
    let path = fixture.tasks.join(TASK_SKELETON_FILE);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(
        super::super::frozen_chain_snapshot_read_only(
            &fixture.project,
            &fixture.prd,
            &fixture.tasks
        )
        .is_err()
    );
    assert!(
        !path.exists(),
        "read-only verification cannot restore bytes"
    );
    assert_eq!(snapshot(&fixture).unwrap(), before);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    let cache = fixture
        .project
        .join(".archon/freeze-cache/skeletons")
        .join(digest);
    std::fs::write(cache, b"{}").unwrap();
    std::fs::remove_file(&path).unwrap();
    let found = envelope(&fixture, "skeleton");
    assert_eq!(found["operational_error"]["kind"], "permanent", "{found}");
    assert!(
        !path.exists(),
        "unauthenticated cache bytes cannot be restored"
    );
}

#[test]
fn frozen_verification_refuted_acceptance_names_the_check_and_retains_its_contract() {
    let mut fixture = project();
    freeze_acceptance(&mut fixture);
    let path = fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    document["acceptance"][0]["judgment"]["verdict"] = "refuted".into();
    let bytes = serde_json::to_vec_pretty(&document).unwrap();
    let digest = archon_workflow::task_set_contract::content_digest(&bytes);
    std::fs::write(&path, bytes).unwrap();
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(&fixture.project, &fixture.tasks);
    for (path, field) in [
        (fixture.tasks.join(ACCEPTANCE_LOCK_FILE), "digest"),
        (pin_path, "acceptance_digest"),
    ] {
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value[field] = digest.clone().into();
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    let found = envelope(&fixture, "acceptance");
    assert!(found["operational_error"].is_null(), "{found}");
    assert_eq!(
        found["policy_findings"][0]["subject"], "AC-X-001",
        "{found}"
    );
    assert_eq!(
        found["policy_findings"][0]["source_path"],
        path.to_string_lossy().replace('\\', "/")
    );
    let retained: serde_json::Value =
        serde_json::from_str(found["report"].as_str().unwrap()).unwrap();
    assert_eq!(retained, document);
}
