//! Apply a PolicyDecision to a BehaviourProposal.
//!
//! Flow:
//! 1. Re-read proposal from DB (concurrency check — must still be Pending).
//! 2. If PendingApproval → store a BehaviourApproval and return.
//! 3. If AutoApplied → create new BehaviourManifestVersion, update proposal status,
//!    log a LearningEvent.
//! 4. If Denied → update proposal status, log a LearningEvent.

use anyhow::Result;
use cozo::DbInstance;

use crate::errors::LearningError;
use crate::manifest;
use crate::models::*;
use crate::store;

#[derive(Debug)]
pub struct ApplyResult {
    pub proposal: BehaviourProposal,
    pub new_version: Option<BehaviourManifestVersion>,
    pub approval: Option<BehaviourApproval>,
}

/// Apply a policy decision to a proposal.
pub fn apply_decision(
    db: &DbInstance,
    proposal_id: &str,
    decision: PolicyDecision,
    new_content: Option<serde_json::Value>,
    approver: Option<&str>,
) -> Result<ApplyResult, LearningError> {
    // Concurrency check: re-read proposal, must still be Pending
    let proposal =
        store::get_behaviour_proposal(db, proposal_id)?.ok_or(LearningError::ProposalNotFound {
            proposal_id: proposal_id.to_string(),
        })?;

    if proposal.status != ProposalStatus::Pending {
        return Err(LearningError::ConcurrentModification {
            expected: ProposalStatus::Pending.as_str().to_string(),
            actual: proposal.status.as_str().to_string(),
        });
    }

    match decision {
        PolicyDecision::PendingApproval => apply_pending_approval(db, &proposal, approver),
        PolicyDecision::AutoApplied => apply_auto(db, &proposal, new_content),
        PolicyDecision::Denied => apply_denied(db, &proposal),
        PolicyDecision::Approved => apply_approved(db, &proposal, new_content, approver),
        PolicyDecision::Rejected => apply_denied(db, &proposal),
    }
}

fn apply_pending_approval(
    db: &DbInstance,
    proposal: &BehaviourProposal,
    approver: Option<&str>,
) -> Result<ApplyResult, LearningError> {
    let approval = BehaviourApproval {
        approval_id: format!(
            "ba-{}",
            uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
        ),
        proposal_id: proposal.proposal_id.clone(),
        approver: approver.unwrap_or("system").to_string(),
        approved: false,
        comment: "Awaiting human review".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    store::insert_approval(db, &approval).map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    Ok(ApplyResult {
        proposal: proposal.clone(),
        new_version: None,
        approval: Some(approval),
    })
}

fn apply_auto(
    db: &DbInstance,
    proposal: &BehaviourProposal,
    new_content: Option<serde_json::Value>,
) -> Result<ApplyResult, LearningError> {
    let apply_content = resolve_apply_content(db, proposal, new_content)?;
    let current = manifest::load_current(db, &proposal.manifest_kind)?;
    let version = create_manifest_version(
        db,
        proposal,
        &apply_content.content,
        current.as_ref().map(|v| v.version_id.as_str()),
        apply_content.is_rollback,
    )?;

    store::update_proposal_status(
        db,
        &proposal.proposal_id,
        &ProposalStatus::Applied,
        &PolicyDecision::AutoApplied,
    )
    .map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    let event_type = if apply_content.is_rollback {
        LearningEventType::ManifestRolledBack
    } else {
        LearningEventType::ManifestApplied
    };
    let mut signal = serde_json::json!({"manifest_kind": proposal.manifest_kind.as_str()});
    if let Some(target_id) = apply_content.rollback_target_id.as_deref() {
        signal["rolled_back_from"] = serde_json::json!(target_id);
    }

    crate::events::record_event(
        db,
        &proposal.workspace_id,
        event_type,
        &proposal.proposal_id,
        Some(&version.version_id),
        signal,
        1.0,
        "",
    )
    .map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    let mut updated = proposal.clone();
    updated.status = ProposalStatus::Applied;
    updated.policy_decision = PolicyDecision::AutoApplied;

    Ok(ApplyResult {
        proposal: updated,
        new_version: Some(version),
        approval: None,
    })
}

fn apply_denied(
    db: &DbInstance,
    proposal: &BehaviourProposal,
) -> Result<ApplyResult, LearningError> {
    store::update_proposal_status(
        db,
        &proposal.proposal_id,
        &ProposalStatus::Denied,
        &PolicyDecision::Denied,
    )
    .map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    crate::events::record_event(
        db,
        &proposal.workspace_id,
        LearningEventType::ManifestDenied,
        &proposal.proposal_id,
        None,
        serde_json::json!({"manifest_kind": proposal.manifest_kind.as_str()}),
        1.0,
        "",
    )
    .map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    let mut updated = proposal.clone();
    updated.status = ProposalStatus::Denied;
    updated.policy_decision = PolicyDecision::Denied;

    Ok(ApplyResult {
        proposal: updated,
        new_version: None,
        approval: None,
    })
}

fn apply_approved(
    db: &DbInstance,
    proposal: &BehaviourProposal,
    new_content: Option<serde_json::Value>,
    approver: Option<&str>,
) -> Result<ApplyResult, LearningError> {
    let apply_content = resolve_apply_content(db, proposal, new_content)?;
    let current = manifest::load_current(db, &proposal.manifest_kind)?;
    let version = create_manifest_version(
        db,
        proposal,
        &apply_content.content,
        current.as_ref().map(|v| v.version_id.as_str()),
        apply_content.is_rollback,
    )?;

    store::update_proposal_status(
        db,
        &proposal.proposal_id,
        &ProposalStatus::Applied,
        &PolicyDecision::Approved,
    )
    .map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    let event_type = if apply_content.is_rollback {
        LearningEventType::ManifestRolledBack
    } else {
        LearningEventType::ManifestApplied
    };
    let mut signal = serde_json::json!({
        "manifest_kind": proposal.manifest_kind.as_str(),
        "approver": approver.unwrap_or("system"),
    });
    if let Some(target_id) = apply_content.rollback_target_id.as_deref() {
        signal["rolled_back_from"] = serde_json::json!(target_id);
    }

    crate::events::record_event(
        db,
        &proposal.workspace_id,
        event_type,
        &proposal.proposal_id,
        Some(&version.version_id),
        signal,
        1.0,
        "",
    )
    .map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    let mut updated = proposal.clone();
    updated.status = ProposalStatus::Applied;
    updated.policy_decision = PolicyDecision::Approved;

    Ok(ApplyResult {
        proposal: updated,
        new_version: Some(version),
        approval: None,
    })
}

struct ApplyContent {
    content: serde_json::Value,
    is_rollback: bool,
    rollback_target_id: Option<String>,
}

fn resolve_apply_content(
    db: &DbInstance,
    proposal: &BehaviourProposal,
    new_content: Option<serde_json::Value>,
) -> Result<ApplyContent, LearningError> {
    validate_proposal_for_apply(proposal)?;

    if let Some(content) = new_content {
        validate_apply_content(&content)?;
        verify_diff_if_possible(db, proposal, content.clone())?;
        return Ok(ApplyContent {
            content,
            is_rollback: false,
            rollback_target_id: None,
        });
    }

    if let Some(target_id) = proposal.proposed_version.strip_prefix("rollback-to-") {
        let target = store::get_manifest_version(db, target_id)?.ok_or(
            LearningError::RollbackTargetUnreachable {
                version_id: target_id.to_string(),
            },
        )?;
        if target.manifest_kind != proposal.manifest_kind {
            return Err(LearningError::Validation {
                message: format!(
                    "rollback target {target_id} is {target_kind}, proposal is {proposal_kind}",
                    target_kind = target.manifest_kind.as_str(),
                    proposal_kind = proposal.manifest_kind.as_str(),
                ),
            });
        }
        return Ok(ApplyContent {
            content: target.content,
            is_rollback: true,
            rollback_target_id: Some(target_id.to_string()),
        });
    }

    if proposal.diff.trim().is_empty() {
        return Err(LearningError::Validation {
            message: format!(
                "proposal {} has no explicit content and no diff-derived content",
                proposal.proposal_id
            ),
        });
    }

    let content = serde_json::json!({
        "manifest_kind": proposal.manifest_kind.as_str(),
        "proposed_version": proposal.proposed_version.clone(),
        "diff": proposal.diff.clone(),
        "evidence_ids": proposal.evidence_ids.clone(),
    });
    validate_apply_content(&content)?;
    Ok(ApplyContent {
        content,
        is_rollback: false,
        rollback_target_id: None,
    })
}

fn validate_proposal_for_apply(proposal: &BehaviourProposal) -> Result<(), LearningError> {
    if proposal.current_version.trim().is_empty() || proposal.current_version == "unresolved" {
        return Err(LearningError::Validation {
            message: format!(
                "proposal {} is missing current_version and is not auto-applicable",
                proposal.proposal_id
            ),
        });
    }
    if proposal.proposed_version.trim().is_empty() {
        return Err(LearningError::Validation {
            message: format!(
                "proposal {} is missing proposed_version",
                proposal.proposal_id
            ),
        });
    }
    Ok(())
}

fn validate_apply_content(content: &serde_json::Value) -> Result<(), LearningError> {
    match content {
        serde_json::Value::Object(map) if !map.is_empty() => Ok(()),
        _ => Err(LearningError::Validation {
            message: "proposal content must be a non-empty JSON object".into(),
        }),
    }
}

fn verify_diff_if_possible(
    db: &DbInstance,
    proposal: &BehaviourProposal,
    new_content: serde_json::Value,
) -> Result<(), LearningError> {
    let trimmed = proposal.diff.trim();
    if !trimmed.starts_with('[') {
        return Ok(());
    }
    let Some(current) = manifest::load_current(db, &proposal.manifest_kind)? else {
        return Ok(());
    };
    manifest::apply_diff(&current.content, trimmed, new_content).map(|_| ())
}

fn create_manifest_version(
    db: &DbInstance,
    proposal: &BehaviourProposal,
    content: &serde_json::Value,
    parent_version_id: Option<&str>,
    is_rollback_target: bool,
) -> Result<BehaviourManifestVersion, LearningError> {
    let version_id = format!(
        "bmv-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
    );
    let version_number = next_version_number(db, proposal.manifest_kind.as_str())?;
    let created_at = chrono::Utc::now().to_rfc3339();

    let version = BehaviourManifestVersion {
        version_id: version_id.clone(),
        manifest_kind: proposal.manifest_kind.clone(),
        version_number,
        content: content.clone(),
        diff: proposal.diff.clone(),
        parent_version_id: parent_version_id.map(|s| s.to_string()),
        created_by_proposal_id: Some(proposal.proposal_id.clone()),
        is_rollback_target,
        created_at: created_at.clone(),
    };

    store::insert_manifest_version(db, &version).map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    Ok(version)
}

fn next_version_number(db: &DbInstance, manifest_kind: &str) -> Result<i64, LearningError> {
    let latest = store::get_latest_manifest_version(db, manifest_kind).map_err(|e| {
        LearningError::Storage {
            message: e.to_string(),
        }
    })?;
    Ok(latest.as_ref().map(|v| v.version_number + 1).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-apply-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
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
}
