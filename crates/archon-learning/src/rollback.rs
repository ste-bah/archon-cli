//! Rollback a manifest version to a previous target.
//!
//! Creates an audited rollback proposal, evaluates policy, and only creates a
//! NEW version when policy allows auto-apply. Approval-required rollbacks leave
//! manifest state untouched until a human decision applies them.
//!
//! Preconditions:
//! - Target version must exist and must belong to the expected manifest_kind.
//! - A rollback creates a proposal for the audit trail.

use anyhow::Result;
use cozo::DbInstance;

use crate::errors::LearningError;
use crate::models::*;
use crate::policy;
use crate::store;

pub struct RollbackResult {
    pub proposal: BehaviourProposal,
    pub new_version: Option<BehaviourManifestVersion>,
    pub approval: Option<BehaviourApproval>,
    pub policy_outcomes: Vec<PolicyOutcome>,
    pub rolled_back_from_version_id: String,
}

/// Execute a rollback to the given target version.
pub fn rollback_to_version(
    db: &DbInstance,
    target_version_id: &str,
    workspace_id: &str,
    reason: &str,
) -> Result<RollbackResult, LearningError> {
    rollback_to_version_with_auto_apply(db, target_version_id, workspace_id, reason, false, 0)
}

/// Execute a rollback using an already-loaded Evidence Engine policy.
pub fn rollback_to_version_with_policy(
    db: &DbInstance,
    target_version_id: &str,
    workspace_id: &str,
    reason: &str,
    effective_policy: &archon_policy::EffectivePolicy,
    recent_incident_count: usize,
) -> Result<RollbackResult, LearningError> {
    let target = store::get_manifest_version(db, target_version_id)?.ok_or(
        LearningError::RollbackTargetUnreachable {
            version_id: target_version_id.to_string(),
        },
    )?;
    let policy_decision = effective_policy
        .learning_auto_apply_decision(
            target.manifest_kind.as_str(),
            target.manifest_kind.default_risk_level().as_str(),
        )
        .allowed;
    rollback_to_version_with_auto_apply(
        db,
        target_version_id,
        workspace_id,
        reason,
        policy_decision,
        recent_incident_count,
    )
}

/// Execute a rollback with an explicit auto-apply permission decision.
pub fn rollback_to_version_with_auto_apply(
    db: &DbInstance,
    target_version_id: &str,
    workspace_id: &str,
    reason: &str,
    allow_auto_apply: bool,
    recent_incident_count: usize,
) -> Result<RollbackResult, LearningError> {
    let target = store::get_manifest_version(db, target_version_id)?.ok_or(
        LearningError::RollbackTargetUnreachable {
            version_id: target_version_id.to_string(),
        },
    )?;

    let manifest_kind = target.manifest_kind.clone();

    let current_head =
        store::get_latest_manifest_version(db, manifest_kind.as_str()).map_err(|e| {
            LearningError::Storage {
                message: e.to_string(),
            }
        })?;

    let proposal = create_rollback_proposal(
        db,
        workspace_id,
        &manifest_kind,
        current_head
            .as_ref()
            .map(|v| v.version_id.as_str())
            .unwrap_or(""),
        target_version_id,
        reason,
    )?;

    let (decision, policy_outcomes) =
        policy::evaluate_proposal(db, &proposal, allow_auto_apply, recent_incident_count)?;

    if decision != PolicyDecision::AutoApplied {
        let apply_result = crate::apply::apply_decision(
            db,
            &proposal.proposal_id,
            decision,
            None,
            Some("system"),
        )?;
        return Ok(RollbackResult {
            proposal: apply_result.proposal,
            new_version: None,
            approval: apply_result.approval,
            policy_outcomes,
            rolled_back_from_version_id: target_version_id.to_string(),
        });
    }

    let version_id = format!(
        "bmv-{}",
        &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
    );
    let version_number = current_head
        .as_ref()
        .map(|v| v.version_number + 1)
        .unwrap_or(1);
    let parent_id = current_head
        .as_ref()
        .map(|v| v.version_id.as_str())
        .unwrap_or("");
    let diff = format!(
        "Rollback to {target}\nReason: {reason}",
        target = target_version_id,
    );
    let created_at = chrono::Utc::now().to_rfc3339();

    let new_version = BehaviourManifestVersion {
        version_id: version_id.clone(),
        manifest_kind: manifest_kind.clone(),
        version_number,
        content: target.content.clone(),
        diff: diff.clone(),
        parent_version_id: if parent_id.is_empty() {
            None
        } else {
            Some(parent_id.to_string())
        },
        created_by_proposal_id: Some(proposal.proposal_id.clone()),
        is_rollback_target: true,
        created_at: created_at.clone(),
    };

    store::insert_manifest_version(db, &new_version).map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    store::update_proposal_status(
        db,
        &proposal.proposal_id,
        &ProposalStatus::Applied,
        &PolicyDecision::AutoApplied,
    )
    .map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    crate::events::record_event(
        db,
        workspace_id,
        LearningEventType::ManifestRolledBack,
        &proposal.proposal_id,
        Some(&version_id),
        serde_json::json!({
            "manifest_kind": manifest_kind.as_str(),
            "rolled_back_from": target_version_id,
            "reason": reason,
        }),
        1.0,
        "",
    )
    .map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    let mut proposal = proposal;
    proposal.status = ProposalStatus::Applied;
    proposal.policy_decision = PolicyDecision::AutoApplied;

    Ok(RollbackResult {
        proposal,
        new_version: Some(new_version),
        approval: None,
        policy_outcomes,
        rolled_back_from_version_id: target_version_id.to_string(),
    })
}

fn create_rollback_proposal(
    db: &DbInstance,
    workspace_id: &str,
    manifest_kind: &BehaviourManifestKind,
    current_version_id: &str,
    target_version_id: &str,
    reason: &str,
) -> Result<BehaviourProposal, LearningError> {
    let created_at = chrono::Utc::now().to_rfc3339();
    let proposal = BehaviourProposal {
        proposal_id: format!(
            "bp-{}",
            &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
        ),
        workspace_id: workspace_id.to_string(),
        manifest_kind: manifest_kind.clone(),
        current_version: current_version_id.to_string(),
        proposed_version: format!("rollback-to-{target_version_id}"),
        diff: format!(
            "Rollback {kind} to version {target}\nReason: {reason}",
            kind = manifest_kind.as_str(),
            target = target_version_id,
        ),
        evidence_ids: vec![target_version_id.to_string()],
        risk_level: manifest_kind.default_risk_level(),
        policy_decision: PolicyDecision::PendingApproval,
        status: ProposalStatus::Pending,
        created_at: created_at.clone(),
    };

    store::insert_behaviour_proposal(db, &proposal).map_err(|e| LearningError::Storage {
        message: e.to_string(),
    })?;

    Ok(proposal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-rollback-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    fn seed_version(
        db: &DbInstance,
        version_id: &str,
        kind: &str,
        version_number: i64,
        content: serde_json::Value,
    ) {
        let v = BehaviourManifestVersion {
            version_id: version_id.to_string(),
            manifest_kind: BehaviourManifestKind::from_str(kind).unwrap(),
            version_number,
            content,
            diff: "seed diff".to_string(),
            parent_version_id: None,
            created_by_proposal_id: Some("bp-seed".to_string()),
            is_rollback_target: false,
            created_at: format!("2026-01-01T00:00:{version_number:02}Z"),
        };
        store::insert_manifest_version(db, &v).unwrap();
    }

    #[test]
    fn test_auto_apply_rollback_creates_new_version() {
        let db = test_db();
        seed_version(
            &db,
            "bmv-v1",
            "RetrievalProfile",
            1,
            serde_json::json!({"weight": 1.0}),
        );

        let result =
            rollback_to_version_with_auto_apply(&db, "bmv-v1", "ws-test", "test rollback", true, 0)
                .unwrap();
        let new_version = result
            .new_version
            .as_ref()
            .expect("low-risk auto-apply should create a rollback version");

        assert_eq!(result.rolled_back_from_version_id, "bmv-v1");
        assert!(new_version.is_rollback_target);
        assert_eq!(new_version.content, serde_json::json!({"weight": 1.0}));
        assert!(new_version.version_number > 1);
        assert!(result.approval.is_none());
        assert_eq!(result.proposal.status, ProposalStatus::Applied);
    }

    #[test]
    fn test_rollback_nonexistent_version_fails() {
        let db = test_db();
        let result = rollback_to_version(&db, "bmv-nonexistent", "ws-test", "should fail");
        assert!(result.is_err());
    }

    #[test]
    fn test_rollback_creates_proposal_audit_trail() {
        let db = test_db();
        seed_version(
            &db,
            "bmv-v1",
            "RetrievalProfile",
            1,
            serde_json::json!({"weight": 0.8}),
        );

        let result = rollback_to_version(&db, "bmv-v1", "ws-test", "audit test").unwrap();

        let proposal = store::get_behaviour_proposal(&db, &result.proposal.proposal_id)
            .unwrap()
            .unwrap();
        assert_eq!(proposal.status, ProposalStatus::Pending);
        assert_eq!(proposal.policy_decision, PolicyDecision::PendingApproval);
        assert_eq!(proposal.current_version, "bmv-v1");
        assert_eq!(
            proposal.manifest_kind,
            BehaviourManifestKind::RetrievalProfile
        );
        assert!(result.new_version.is_none());
        assert!(result.approval.is_some());
    }

    #[test]
    fn test_default_rollback_does_not_mutate_manifest_without_policy_allowance() {
        let db = test_db();
        seed_version(
            &db,
            "bmv-v1",
            "RetrievalProfile",
            1,
            serde_json::json!({"weight": 0.8}),
        );
        seed_version(
            &db,
            "bmv-v2",
            "RetrievalProfile",
            2,
            serde_json::json!({"weight": 0.2}),
        );

        let result = rollback_to_version(&db, "bmv-v1", "ws-test", "policy gate").unwrap();

        assert!(result.new_version.is_none());
        assert!(result.approval.is_some());
        assert_eq!(result.proposal.status, ProposalStatus::Pending);
        assert_eq!(result.proposal.current_version, "bmv-v2");

        let versions = store::list_manifest_version_history(&db, "RetrievalProfile").unwrap();
        assert_eq!(versions.len(), 2);
        assert!(
            store::list_learning_events_by_type(&db, "ManifestRolledBack")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_rollback_records_policy_outcomes() {
        let db = test_db();
        seed_version(
            &db,
            "bmv-v1",
            "RetrievalProfile",
            1,
            serde_json::json!({"weight": 0.8}),
        );

        let result = rollback_to_version(&db, "bmv-v1", "ws-test", "policy evidence").unwrap();

        assert!(
            result
                .policy_outcomes
                .iter()
                .any(|o| o.rule_name == "fallback_pending_approval")
        );
        let stored =
            store::list_policy_decisions_for_proposal(&db, &result.proposal.proposal_id).unwrap();
        assert_eq!(stored.len(), result.policy_outcomes.len());
        assert_eq!(stored[0].outcome, PolicyDecision::PendingApproval);
    }

    #[test]
    fn test_high_risk_rollback_requires_approval_even_when_auto_apply_enabled() {
        let db = test_db();
        seed_version(
            &db,
            "bmv-gates-v1",
            "PipelineGates",
            1,
            serde_json::json!({"required": ["tests"]}),
        );

        let result = rollback_to_version_with_auto_apply(
            &db,
            "bmv-gates-v1",
            "ws-test",
            "high-risk rollback",
            true,
            0,
        )
        .unwrap();

        assert!(result.new_version.is_none());
        assert!(result.approval.is_some());
        assert_eq!(result.proposal.status, ProposalStatus::Pending);
        assert!(
            result
                .policy_outcomes
                .iter()
                .any(|o| o.rule_name == "high_risk_requires_approval")
        );
        let versions = store::list_manifest_version_history(&db, "PipelineGates").unwrap();
        assert_eq!(versions.len(), 1);
    }

    #[test]
    fn test_rollback_logs_learning_event() {
        let db = test_db();
        seed_version(
            &db,
            "bmv-v1",
            "RetrievalProfile",
            1,
            serde_json::json!({"weight": 0.9}),
        );

        rollback_to_version_with_auto_apply(&db, "bmv-v1", "ws-test", "event log test", true, 0)
            .unwrap();

        let events = store::list_learning_events_by_type(&db, "ManifestRolledBack").unwrap();
        assert!(!events.is_empty());
    }
}
