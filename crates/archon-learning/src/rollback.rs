//! Rollback a manifest version to a previous target.
//!
//! Creates a NEW version (never mutates existing rows) whose content points to
//! the rollback target. Sets is_rollback_target=true on the new version.
//!
//! Preconditions:
//! - Target version must exist and must belong to the expected manifest_kind.
//! - A rollback creates a proposal for the audit trail.

use anyhow::Result;
use cozo::DbInstance;

use crate::errors::LearningError;
use crate::models::*;
use crate::store;

pub struct RollbackResult {
    pub proposal: BehaviourProposal,
    pub new_version: BehaviourManifestVersion,
    pub rolled_back_from_version_id: String,
}

/// Execute a rollback to the given target version.
pub fn rollback_to_version(
    db: &DbInstance,
    target_version_id: &str,
    workspace_id: &str,
    reason: &str,
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

    let proposal =
        create_rollback_proposal(db, workspace_id, &manifest_kind, target_version_id, reason)?;

    let version_id = format!(
        "bmv-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
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
        new_version,
        rolled_back_from_version_id: target_version_id.to_string(),
    })
}

fn create_rollback_proposal(
    db: &DbInstance,
    workspace_id: &str,
    manifest_kind: &BehaviourManifestKind,
    target_version_id: &str,
    reason: &str,
) -> Result<BehaviourProposal, LearningError> {
    let created_at = chrono::Utc::now().to_rfc3339();
    let proposal = BehaviourProposal {
        proposal_id: format!(
            "bp-{}",
            uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
        ),
        workspace_id: workspace_id.to_string(),
        manifest_kind: manifest_kind.clone(),
        current_version: String::new(),
        proposed_version: format!("rollback-to-{target_version_id}"),
        diff: format!(
            "Rollback {kind} to version {target}\nReason: {reason}",
            kind = manifest_kind.as_str(),
            target = target_version_id,
        ),
        evidence_ids: vec![target_version_id.to_string()],
        risk_level: manifest_kind.default_risk_level(),
        policy_decision: PolicyDecision::AutoApplied,
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
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        store::insert_manifest_version(db, &v).unwrap();
    }

    #[test]
    fn test_rollback_creates_new_version() {
        let db = test_db();
        seed_version(
            &db,
            "bmv-v1",
            "RetrievalProfile",
            1,
            serde_json::json!({"weight": 1.0}),
        );

        let result = rollback_to_version(&db, "bmv-v1", "ws-test", "test rollback").unwrap();

        assert_eq!(result.rolled_back_from_version_id, "bmv-v1");
        assert!(result.new_version.is_rollback_target);
        assert_eq!(
            result.new_version.content,
            serde_json::json!({"weight": 1.0})
        );
        assert!(result.new_version.version_number > 1);
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
        assert_eq!(proposal.status, ProposalStatus::Applied);
        assert_eq!(
            proposal.manifest_kind,
            BehaviourManifestKind::RetrievalProfile
        );
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

        rollback_to_version(&db, "bmv-v1", "ws-test", "event log test").unwrap();

        let events = store::list_learning_events_by_type(&db, "ManifestRolledBack").unwrap();
        assert!(!events.is_empty());
    }
}
