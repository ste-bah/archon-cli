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
            &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
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
        &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
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
#[path = "apply_tests.rs"]
mod apply_tests;
