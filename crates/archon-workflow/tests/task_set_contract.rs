use std::collections::BTreeSet;
use std::fs;

use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, AcceptanceLock,
    AcceptancePin, FreezeGateMode, FreezeGateStamp, GapPolicy, ResidualGapRecord,
    acceptance_policy_findings, validate_acceptance_bundle, validate_acceptance_contract,
    validate_acceptance_structure, validate_residual_gaps,
};

fn acceptance(command: &str, cwd: &str, gap_permitted: bool) -> String {
    format!(
        r#"{{
          "schema_version": 1,
          "prd": {{"path":"prds/PRD-X.md","digest":"prd-digest"}},
          "gap_policy": {{
            "permitted_acceptance_ids": {permitted},
            "forbidden_phrases": ["later", "TBD"],
            "required_fields": ["id","acceptance_id","area","description","impact","fail_closed_behavior","owner","created_at","fail_closed_check"]
          }},
          "acceptance": [{{
            "id":"AC-X-001",
            "criterion":"The result exists and is valid",
            "check":{{"kind":"command","command":{command:?},"cwd":{cwd:?}}},
            "gap_permitted":{gap_permitted},
            "judgment":{{"verdict":"accepted","counterexample":"attempted state","reason":"predicate rejects it","host_call_id":"judge-1"}}
          }}],
          "supplementary": []
        }}"#,
        permitted = if gap_permitted {
            r#"["AC-X-001"]"#
        } else {
            "[]"
        },
    )
}

fn clean_stamp() -> FreezeGateStamp {
    FreezeGateStamp {
        mode: FreezeGateMode::Enforce,
        finding_count: 0,
        findings_digest: archon_workflow::task_set_contract::empty_gate_findings_digest(),
        binary_commit: "test-revision".into(),
        evaluated_at: "2026-08-26T18:30:00Z".into(),
    }
}

fn expected() -> BTreeSet<String> {
    ["AC-X-001".to_string()].into_iter().collect()
}

#[test]
fn cwd_is_a_closed_host_enum_not_an_arbitrary_path() {
    let literal = acceptance("jq -e '.ready == true' out.json", "/tmp/escape", false);
    assert!(serde_json::from_str::<AcceptanceContract>(&literal).is_err());

    let trusted = acceptance("jq -e '.ready == true' out.json", "project_root", false);
    let parsed: AcceptanceContract = serde_json::from_str(&trusted).expect("trusted cwd");
    assert!(validate_acceptance_contract(&parsed, &expected(), true).is_ok());
}

#[test]
fn every_acceptance_id_is_exactly_once_and_supplementary_ids_are_distinct() {
    let raw = acceptance("jq -e '.ready == true' out.json", "project_root", false);
    let parsed: AcceptanceContract = serde_json::from_str(&raw).unwrap();
    assert!(validate_acceptance_contract(&parsed, &expected(), true).is_ok());

    let mut unknown_permission = parsed.clone();
    unknown_permission
        .gap_policy
        .permitted_acceptance_ids
        .insert("AC-X-999".into());
    let error = validate_acceptance_contract(&unknown_permission, &expected(), true)
        .unwrap_err()
        .to_string();
    assert!(error.contains("AC-X-999"), "{error}");
    assert!(
        error.contains("gap_policy.permitted_acceptance_ids")
            && error.contains("remove each unknown id"),
        "{error}"
    );

    let mut missing = parsed.clone();
    missing.acceptance.clear();
    assert!(validate_acceptance_contract(&missing, &expected(), true).is_err());

    let mut collision = parsed.clone();
    let mut supplementary = collision.acceptance[0].clone();
    supplementary.id = "AC-X-001".into();
    collision.supplementary.push(supplementary);
    assert!(validate_acceptance_contract(&collision, &expected(), true).is_err());

    let mut distinct = parsed;
    let mut supplementary = distinct.acceptance[0].clone();
    supplementary.id = "SUP-X-001".into();
    distinct.supplementary.push(supplementary);
    assert!(validate_acceptance_contract(&distinct, &expected(), true).is_ok());
}

#[test]
fn portable_lock_hashes_exact_file_bytes_and_pin_detects_reminting() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let bytes = acceptance("jq -e '.ready == true' out.json", "project_root", false).into_bytes();
    fs::write(root.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
    let digest = blake3::hash(&bytes).to_hex().to_string();
    let lock = AcceptanceLock {
        algorithm: "blake3".into(),
        digest: digest.clone(),
        gate: clean_stamp(),
    };
    fs::write(
        root.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&lock).unwrap(),
    )
    .unwrap();
    let pin = AcceptancePin {
        task_root: root.canonicalize().unwrap().display().to_string(),
        acceptance_digest: digest.clone(),
        freeze_event_id: "freeze-1".into(),
        acceptance_gate: clean_stamp(),
        skeleton_digest: None,
        skeleton_gate: None,
    };
    assert!(validate_acceptance_bundle(root, Some(&pin), &expected()).is_ok());

    let replacement =
        acceptance("jq -e '.other == true' out.json", "project_root", false).into_bytes();
    fs::write(root.join(ACCEPTANCE_CONTRACT_FILE), &replacement).unwrap();
    let replacement_digest = blake3::hash(&replacement).to_hex().to_string();
    let replacement_lock = AcceptanceLock {
        algorithm: "blake3".into(),
        digest: replacement_digest,
        gate: clean_stamp(),
    };
    fs::write(
        root.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&replacement_lock).unwrap(),
    )
    .unwrap();
    let error = validate_acceptance_bundle(root, Some(&pin), &expected())
        .unwrap_err()
        .to_string();
    assert!(error.contains(&digest), "{error}");
    assert!(error.contains("freeze-1"), "{error}");
    assert!(error.contains("restore the frozen version"), "{error}");
}

fn gap(acceptance_id: &str, description: &str, command: &str) -> ResidualGapRecord {
    ResidualGapRecord {
        id: "GAP-X-001".into(),
        acceptance_id: acceptance_id.into(),
        area: "provider".into(),
        description: description.into(),
        impact: "output cannot be trusted".into(),
        fail_closed_behavior: "the host rejects promotion".into(),
        owner: "owner".into(),
        created_at: "2026-08-25T00:00:00Z".into(),
        fail_closed_check: command.into(),
    }
}

#[test]
fn residual_gaps_require_permission_concrete_fields_and_falsifiable_checks() {
    let policy = GapPolicy {
        permitted_acceptance_ids: ["AC-X-001".into()].into_iter().collect(),
        forbidden_phrases: vec!["later".into(), "TBD".into()],
        required_fields: vec![
            "id".into(),
            "acceptance_id".into(),
            "area".into(),
            "description".into(),
            "impact".into(),
            "fail_closed_behavior".into(),
            "owner".into(),
            "created_at".into(),
            "fail_closed_check".into(),
        ],
    };
    assert!(
        validate_residual_gaps(
            &policy,
            &[gap(
                "AC-X-999",
                "concrete",
                "jq -e '.blocked == true' state.json"
            )]
        )
        .is_err()
    );
    let vague = validate_residual_gaps(
        &policy,
        &[gap(
            "AC-X-001",
            "finish later",
            "jq -e '.blocked == true' state.json",
        )],
    )
    .unwrap_err()
    .to_string();
    assert!(
        vague.contains("description") && vague.contains("later"),
        "{vague}"
    );
    assert!(validate_residual_gaps(&policy, &[gap("AC-X-001", "concrete", "true")]).is_err());
    assert!(
        validate_residual_gaps(
            &policy,
            &[gap(
                "AC-X-001",
                "concrete",
                "jq -e '.blocked == true' state.json"
            )]
        )
        .is_ok()
    );
}

#[test]
fn pin_binds_the_canonical_task_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("tasks-a");
    let other = temp.path().join("tasks-b");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&other).unwrap();
    let bytes = acceptance("jq -e '.ready == true' out.json", "project_root", false).into_bytes();
    fs::write(root.join(ACCEPTANCE_CONTRACT_FILE), &bytes).unwrap();
    let digest = blake3::hash(&bytes).to_hex().to_string();
    fs::write(
        root.join(ACCEPTANCE_LOCK_FILE),
        serde_json::to_vec_pretty(&AcceptanceLock {
            algorithm: "blake3".into(),
            digest: digest.clone(),
            gate: clean_stamp(),
        })
        .unwrap(),
    )
    .unwrap();
    let pin = AcceptancePin {
        task_root: other.canonicalize().unwrap().display().to_string(),
        acceptance_digest: digest,
        freeze_event_id: "freeze-root".into(),
        acceptance_gate: clean_stamp(),
        skeleton_digest: None,
        skeleton_gate: None,
    };
    let error = validate_acceptance_bundle(&root, Some(&pin), &expected())
        .unwrap_err()
        .to_string();
    assert!(error.contains("task_root"), "{error}");
    assert!(error.contains("freeze-root"), "{error}");
    assert!(
        error.contains("re-run `workflow freeze-acceptance`"),
        "{error}"
    );
}

#[test]
fn freeze_records_require_gate_provenance() {
    let unstamped_lock = r#"{"algorithm":"blake3","digest":"abc"}"#;
    assert!(
        serde_json::from_str::<AcceptanceLock>(unstamped_lock).is_err(),
        "an unstamped lock must not look like a policy-clean freeze"
    );
    let unstamped_pin = r#"{
      "task_root":"/tmp/tasks",
      "acceptance_digest":"abc",
      "freeze_event_id":"freeze-1"
    }"#;
    assert!(
        serde_json::from_str::<AcceptancePin>(unstamped_pin).is_err(),
        "an unstamped pin must require re-freezing with the current binary"
    );
}

#[test]
fn structural_validation_keeps_weak_checks_as_policy_findings() {
    let raw = acceptance("true", "project_root", false);
    let mut parsed: AcceptanceContract = serde_json::from_str(&raw).unwrap();
    parsed.acceptance[0].judgment.verdict =
        archon_workflow::task_set_contract::JudgeDecision::Refuted;
    parsed.acceptance[0].judgment.reason = "passing false state".into();

    validate_acceptance_structure(&parsed, &expected(), true)
        .expect("schema and judgment shape are structurally complete");
    let findings = acceptance_policy_findings(&parsed);
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("does not judge it")),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.message.contains("refuted")),
        "{findings:?}"
    );
    assert!(validate_acceptance_contract(&parsed, &expected(), true).is_err());
}

#[test]
fn gate_stamp_invariants_reject_mode_count_digest_laundering() {
    use archon_workflow::task_set_contract::validate_gate_stamp;

    let empty_digest = archon_workflow::task_set_contract::empty_gate_findings_digest();
    let finding_digest = blake3::hash(b"finding").to_hex().to_string();

    for (label, stamp) in [
        (
            "enforce with findings",
            FreezeGateStamp {
                mode: FreezeGateMode::Enforce,
                finding_count: 1,
                findings_digest: finding_digest.clone(),
                ..clean_stamp()
            },
        ),
        (
            "zero count with finding digest",
            FreezeGateStamp {
                mode: FreezeGateMode::Observe,
                finding_count: 0,
                findings_digest: finding_digest.clone(),
                ..clean_stamp()
            },
        ),
        (
            "positive count with empty digest",
            FreezeGateStamp {
                mode: FreezeGateMode::Observe,
                finding_count: 1,
                findings_digest: empty_digest.clone(),
                ..clean_stamp()
            },
        ),
    ] {
        let error = validate_gate_stamp(&stamp, label).unwrap_err().to_string();
        assert!(error.contains("re-freeze"), "{label}: {error}");
    }

    assert!(validate_gate_stamp(&clean_stamp(), "clean").is_ok());
    assert!(
        validate_gate_stamp(
            &FreezeGateStamp {
                mode: FreezeGateMode::Observe,
                finding_count: 1,
                findings_digest: finding_digest,
                ..clean_stamp()
            },
            "observed",
        )
        .is_ok()
    );
}
