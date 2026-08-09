use super::*;

fn test_db() -> crate::cozo_guard::TestDb {
    crate::cozo_guard::test_sqlite_db("test-apply")
}

fn make_pending_proposal(db: &DbInstance) -> BehaviourProposal {
    let p = BehaviourProposal {
        proposal_id: "test-prop-apply".to_string(),
        workspace_id: "ws-test".to_string(),
        manifest_kind: BehaviourManifestKind::RetrievalProfile,
        current_version: "none".to_string(),
        proposed_version: "v2".to_string(),
        diff: "test diff".to_string(),
        evidence_ids: vec![],
        risk_level: RiskLevel::Low,
        policy_decision: PolicyDecision::PendingApproval,
        status: ProposalStatus::Pending,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    store::insert_behaviour_proposal(db, &p).unwrap();
    p
}

fn seed_manifest_version(
    db: &DbInstance,
    version_id: &str,
    kind: BehaviourManifestKind,
    version_number: i64,
    content: serde_json::Value,
) {
    let version = BehaviourManifestVersion {
        version_id: version_id.to_string(),
        manifest_kind: kind,
        version_number,
        content,
        diff: "seed".to_string(),
        parent_version_id: None,
        created_by_proposal_id: None,
        is_rollback_target: false,
        created_at: format!("2026-01-01T00:00:{version_number:02}Z"),
    };
    store::insert_manifest_version(db, &version).unwrap();
}

#[test]
fn test_apply_auto_creates_version_and_updates_status() {
    let db = test_db();
    make_pending_proposal(&db);

    let result = apply_decision(
        &db,
        "test-prop-apply",
        PolicyDecision::AutoApplied,
        Some(serde_json::json!({"weight": 0.5})),
        None,
    )
    .unwrap();

    assert_eq!(result.proposal.status, ProposalStatus::Applied);
    assert!(result.new_version.is_some());
    assert_eq!(
        result.new_version.unwrap().content,
        serde_json::json!({"weight": 0.5})
    );
}

#[test]
fn test_apply_rejects_empty_content() {
    let db = test_db();
    make_pending_proposal(&db);

    let result = apply_decision(
        &db,
        "test-prop-apply",
        PolicyDecision::AutoApplied,
        Some(serde_json::json!({})),
        None,
    );

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("non-empty JSON object")
    );
}

#[test]
fn test_apply_rejects_missing_current_version() {
    let db = test_db();
    let mut proposal = make_pending_proposal(&db);
    proposal.proposal_id = "missing-current".into();
    proposal.current_version = String::new();
    store::insert_behaviour_proposal(&db, &proposal).unwrap();

    let result = apply_decision(
        &db,
        "missing-current",
        PolicyDecision::AutoApplied,
        Some(serde_json::json!({"weight": 0.5})),
        None,
    );

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("current_version"));
}

#[test]
fn test_apply_wires_json_patch_diff_validation() {
    let db = test_db();
    seed_manifest_version(
        &db,
        "bmv-current",
        BehaviourManifestKind::RetrievalProfile,
        1,
        serde_json::json!({"weight": 0.4}),
    );
    let mut proposal = make_pending_proposal(&db);
    proposal.proposal_id = "json-patch-prop".into();
    proposal.current_version = "bmv-current".into();
    proposal.diff = r#"[{"op":"replace","path":"/weight","value":0.7}]"#.into();
    store::insert_behaviour_proposal(&db, &proposal).unwrap();

    let result = apply_decision(
        &db,
        "json-patch-prop",
        PolicyDecision::AutoApplied,
        Some(serde_json::json!({"weight": 0.9})),
        None,
    );

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("inconsistent"));
}

#[test]
fn test_apply_denied_updates_status() {
    let db = test_db();
    make_pending_proposal(&db);

    let result =
        apply_decision(&db, "test-prop-apply", PolicyDecision::Denied, None, None).unwrap();

    assert_eq!(result.proposal.status, ProposalStatus::Denied);
}

#[test]
fn test_apply_non_pending_proposal_fails() {
    let db = test_db();
    let mut p = make_pending_proposal(&db);
    p.status = ProposalStatus::Applied;
    store::insert_behaviour_proposal(&db, &p).unwrap();

    let result = apply_decision(
        &db,
        "test-prop-apply",
        PolicyDecision::AutoApplied,
        None,
        None,
    );
    assert!(result.is_err());
}

#[test]
fn test_apply_logs_learning_event() {
    let db = test_db();
    make_pending_proposal(&db);

    apply_decision(
        &db,
        "test-prop-apply",
        PolicyDecision::AutoApplied,
        Some(serde_json::json!({"weight": 0.5})),
        None,
    )
    .unwrap();

    // Verify a ManifestApplied learning event was logged
    let events = store::list_learning_events_by_type(&db, "ManifestApplied").unwrap();
    assert!(!events.is_empty(), "ManifestApplied event should be logged");
    assert_eq!(events[0].source_artifact_id, "test-prop-apply");
    assert!(events[0].confidence > 0.0);
}

#[test]
fn test_apply_rejects_concurrent_modification() {
    let db = test_db();
    make_pending_proposal(&db);

    // First application succeeds
    apply_decision(
        &db,
        "test-prop-apply",
        PolicyDecision::AutoApplied,
        Some(serde_json::json!({"weight": 0.5})),
        None,
    )
    .unwrap();

    // Second application must fail — proposal is no longer Pending
    let result = apply_decision(
        &db,
        "test-prop-apply",
        PolicyDecision::AutoApplied,
        Some(serde_json::json!({"weight": 0.3})),
        None,
    );

    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_msg = format!("{err}");
    assert!(
        err_msg.contains("Applied") || err_msg.contains("concurrent"),
        "error must indicate concurrent modification, got: {err_msg}"
    );
}

#[test]
fn test_approval_flow_creates_row_and_calls_apply() {
    let db = test_db();
    make_pending_proposal(&db);

    let result = apply_decision(
        &db,
        "test-prop-apply",
        PolicyDecision::PendingApproval,
        None,
        Some("human-reviewer"),
    )
    .unwrap();

    // Verify an approval record was created
    assert!(result.approval.is_some());
    let approval = result.approval.unwrap();
    assert_eq!(approval.proposal_id, "test-prop-apply");
    assert_eq!(approval.approver, "human-reviewer");
    assert!(!approval.approved); // Still pending human decision
    assert!(!approval.approval_id.is_empty());

    // No version should be created for PendingApproval
    assert!(result.new_version.is_none());
}

#[test]
fn test_approved_rollback_proposal_uses_target_content() {
    let db = test_db();
    seed_manifest_version(
        &db,
        "bmv-v1",
        BehaviourManifestKind::RetrievalProfile,
        1,
        serde_json::json!({"weight": 0.9}),
    );
    seed_manifest_version(
        &db,
        "bmv-v2",
        BehaviourManifestKind::RetrievalProfile,
        2,
        serde_json::json!({"weight": 0.1}),
    );

    let proposal = BehaviourProposal {
        proposal_id: "rollback-prop-apply".to_string(),
        workspace_id: "ws-test".to_string(),
        manifest_kind: BehaviourManifestKind::RetrievalProfile,
        current_version: "bmv-v2".to_string(),
        proposed_version: "rollback-to-bmv-v1".to_string(),
        diff: "rollback test".to_string(),
        evidence_ids: vec!["bmv-v1".to_string()],
        risk_level: RiskLevel::Low,
        policy_decision: PolicyDecision::PendingApproval,
        status: ProposalStatus::Pending,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    store::insert_behaviour_proposal(&db, &proposal).unwrap();

    let result = apply_decision(
        &db,
        "rollback-prop-apply",
        PolicyDecision::Approved,
        None,
        Some("human-reviewer"),
    )
    .unwrap();
    let version = result
        .new_version
        .expect("approved rollback should create a version");

    assert!(version.is_rollback_target);
    assert_eq!(version.content, serde_json::json!({"weight": 0.9}));
    assert_eq!(version.parent_version_id.as_deref(), Some("bmv-v2"));

    let events = store::list_learning_events_by_type(&db, "ManifestRolledBack").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].signal["rolled_back_from"], "bmv-v1");
}

#[test]
fn test_full_governed_loop_event_to_apply_to_rollback() {
    let db = test_db();

    // 1. Record an outcome signal (learning event)
    let event = crate::events::record_event(
        &db,
        "ws-loop",
        crate::models::LearningEventType::SourceContradicted,
        "source-loop",
        None,
        serde_json::json!({"contradiction": "test data"}),
        0.9,
        "",
    )
    .unwrap();
    assert!(!event.event_id.is_empty());

    // 2. Generate proposals from events — need 3 SourceContradicted for same source
    crate::events::record_event(
        &db,
        "ws-loop",
        crate::models::LearningEventType::SourceContradicted,
        "source-loop",
        None,
        serde_json::json!({"contradiction": "test data 2"}),
        0.9,
        "",
    )
    .unwrap();
    crate::events::record_event(
        &db,
        "ws-loop",
        crate::models::LearningEventType::SourceContradicted,
        "source-loop",
        None,
        serde_json::json!({"contradiction": "test data 3"}),
        0.9,
        "",
    )
    .unwrap();

    let all_events = store::list_all_learning_events(&db).unwrap();
    let proposals = crate::proposal::generate_proposals_for_store(&db, &all_events).unwrap();
    assert!(
        !proposals.is_empty(),
        "3 contradictions should trigger a proposal"
    );

    let proposal = &proposals[0];
    store::insert_behaviour_proposal(&db, proposal).unwrap();

    // 3. Run policy evaluation
    let (decision, _outcomes) = crate::policy::evaluate_proposal(
        &db, proposal, true, // allow auto-apply
        0,    // no recent incidents
    )
    .unwrap();
    assert_eq!(decision, PolicyDecision::AutoApplied);

    // 4. Apply the decision
    let apply_result = apply_decision(
        &db,
        &proposal.proposal_id,
        decision,
        Some(serde_json::json!({"weight": 0.7})),
        None,
    )
    .unwrap();
    assert_eq!(apply_result.proposal.status, ProposalStatus::Applied);
    assert!(apply_result.new_version.is_some());
    let version_id = apply_result
        .new_version
        .as_ref()
        .unwrap()
        .version_id
        .clone();

    // 5. Rollback the applied version
    let rollback_result = crate::rollback::rollback_to_version_with_auto_apply(
        &db,
        &version_id,
        "ws-loop",
        "integration test rollback",
        true,
        0,
    )
    .unwrap();
    assert!(
        rollback_result
            .new_version
            .as_ref()
            .expect("integration rollback should auto-apply")
            .is_rollback_target
    );

    // 6. Verify the full audit trail
    let all_events = store::list_all_learning_events(&db).unwrap();
    let manifest_events: Vec<_> = all_events
        .iter()
        .filter(|e| {
            matches!(
                e.event_type,
                crate::models::LearningEventType::ManifestApplied
                    | crate::models::LearningEventType::ManifestRolledBack
            )
        })
        .collect();
    assert!(
        manifest_events.len() >= 2,
        "should have ManifestApplied + ManifestRolledBack events"
    );
}
