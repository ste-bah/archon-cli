//! CozoDB CRUD operations for governed-learning relations.
//!
//! Follows the established read-modify-write pattern from archon-docs::store.
//! :put requires values for ALL non-default columns.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::errors::COZO_RELATION_NOT_FOUND;
use crate::models::*;

// ── LearningEvent ──────────────────────────────────────────────────────────────

pub fn insert_learning_event(db: &DbInstance, event: &LearningEvent) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event.event_id.as_str()));
    params.insert("wid".into(), DataValue::from(event.workspace_id.as_str()));
    params.insert("et".into(), DataValue::from(event.event_type.as_str()));
    params.insert("sid".into(), DataValue::from(event.source_artifact_id.as_str()));
    params.insert(
        "oid".into(),
        DataValue::from(event.outcome_artifact_id.as_deref().unwrap_or("")),
    );
    params.insert("sig".into(), DataValue::from(event.signal.to_string().as_str()));
    params.insert("cf".into(), DataValue::from(event.confidence as f64));
    params.insert("prid".into(), DataValue::from(event.provenance_record_id.as_str()));
    params.insert("ca".into(), DataValue::from(event.created_at.as_str()));

    db.run_script(
        "?[event_id, workspace_id, event_type, source_artifact_id, \
         outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
         <- [[$eid, $wid, $et, $sid, $oid, $sig, $cf, $prid, $ca]] \
         :put learning_events { event_id => workspace_id, event_type, \
         source_artifact_id, outcome_artifact_id, signal, confidence, \
         provenance_record_id, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert learning_events failed: {e}"))?;
    Ok(())
}

pub fn get_learning_event(db: &DbInstance, event_id: &str) -> Result<Option<LearningEvent>> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event_id));

    let result = db
        .run_script(
            "?[event_id, workspace_id, event_type, source_artifact_id, \
             outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
             := *learning_events{event_id, workspace_id, event_type, \
             source_artifact_id, outcome_artifact_id, signal, confidence, \
             provenance_record_id, created_at}, event_id = $eid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains(COZO_RELATION_NOT_FOUND) {
                return anyhow::anyhow!("{msg}");
            }
            anyhow::anyhow!("get learning_event failed: {msg}")
        })?;

    if result.rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(row_to_learning_event(&result.rows[0])))
}

pub fn list_learning_events_since(
    db: &DbInstance,
    since: &str,
    event_type_filter: Option<&str>,
) -> Result<Vec<LearningEvent>> {
    let script = if let Some(et) = event_type_filter {
        let mut params = BTreeMap::new();
        params.insert("since".into(), DataValue::from(since));
        params.insert("et".into(), DataValue::from(et));
        db.run_script(
            "?[event_id, workspace_id, event_type, source_artifact_id, \
             outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
             := *learning_events{event_id, workspace_id, event_type, \
             source_artifact_id, outcome_artifact_id, signal, confidence, \
             provenance_record_id, created_at}, \
             created_at >= $since, event_type = $et",
            params,
            ScriptMutability::Immutable,
        )
    } else {
        let mut params = BTreeMap::new();
        params.insert("since".into(), DataValue::from(since));
        db.run_script(
            "?[event_id, workspace_id, event_type, source_artifact_id, \
             outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
             := *learning_events{event_id, workspace_id, event_type, \
             source_artifact_id, outcome_artifact_id, signal, confidence, \
             provenance_record_id, created_at}, \
             created_at >= $since",
            params,
            ScriptMutability::Immutable,
        )
    };

    let result = script.map_err(|e| anyhow::anyhow!("list learning_events failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| row_to_learning_event(row))
        .collect())
}

pub fn list_all_learning_events(db: &DbInstance) -> Result<Vec<LearningEvent>> {
    let result = db
        .run_script(
            "?[event_id, workspace_id, event_type, source_artifact_id, \
             outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
             := *learning_events{event_id, workspace_id, event_type, \
             source_artifact_id, outcome_artifact_id, signal, confidence, \
             provenance_record_id, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list all learning_events failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| row_to_learning_event(row))
        .collect())
}

pub fn list_learning_events_by_type(
    db: &DbInstance,
    event_type: &str,
) -> Result<Vec<LearningEvent>> {
    let mut params = BTreeMap::new();
    params.insert("et".into(), DataValue::from(event_type));

    let result = db
        .run_script(
            "?[event_id, workspace_id, event_type, source_artifact_id, \
             outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
             := *learning_events{event_id, workspace_id, event_type, \
             source_artifact_id, outcome_artifact_id, signal, confidence, \
             provenance_record_id, created_at}, event_type = $et",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list learning_events by type failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| row_to_learning_event(row))
        .collect())
}

/// Raw list helper for tests.
pub fn list_all_learning_events_raw(db: &DbInstance) -> Result<Vec<Vec<DataValue>>> {
    let result = db
        .run_script(
            "?[event_id, workspace_id, event_type, source_artifact_id, \
             outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
             := *learning_events{event_id, workspace_id, event_type, \
             source_artifact_id, outcome_artifact_id, signal, confidence, \
             provenance_record_id, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list all learning_events raw failed: {e}"))?;
    Ok(result.rows)
}

fn row_to_learning_event(row: &[DataValue]) -> LearningEvent {
    LearningEvent {
        event_id: row[0].get_str().unwrap_or("").to_string(),
        workspace_id: row[1].get_str().unwrap_or("").to_string(),
        event_type: LearningEventType::from_str(row[2].get_str().unwrap_or(""))
            .unwrap_or(LearningEventType::FalseCompletionDetected),
        source_artifact_id: row[3].get_str().unwrap_or("").to_string(),
        outcome_artifact_id: {
            let s = row[4].get_str().unwrap_or("");
            if s.is_empty() { None } else { Some(s.to_string()) }
        },
        signal: {
            let s = row[5].get_str().unwrap_or("{}");
            serde_json::from_str(s).unwrap_or(serde_json::Value::Object(Default::default()))
        },
        confidence: row[6].get_float().unwrap_or(0.5) as f32,
        provenance_record_id: row[7].get_str().unwrap_or("").to_string(),
        created_at: row[8].get_str().unwrap_or("").to_string(),
    }
}

// ── BehaviourProposal ──────────────────────────────────────────────────────────

pub fn insert_behaviour_proposal(db: &DbInstance, proposal: &BehaviourProposal) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal.proposal_id.as_str()));
    params.insert("wid".into(), DataValue::from(proposal.workspace_id.as_str()));
    params.insert("mk".into(), DataValue::from(proposal.manifest_kind.as_str()));
    params.insert("cv".into(), DataValue::from(proposal.current_version.as_str()));
    params.insert("pv".into(), DataValue::from(proposal.proposed_version.as_str()));
    params.insert("diff".into(), DataValue::from(proposal.diff.as_str()));
    params.insert(
        "evids".into(),
        DataValue::from(serde_json::to_string(&proposal.evidence_ids).unwrap().as_str()),
    );
    params.insert("rl".into(), DataValue::from(proposal.risk_level.as_str()));
    params.insert("pd".into(), DataValue::from(proposal.policy_decision.as_str()));
    params.insert("status".into(), DataValue::from(proposal.status.as_str()));
    params.insert("ca".into(), DataValue::from(proposal.created_at.as_str()));

    db.run_script(
        "?[proposal_id, workspace_id, manifest_kind, current_version, \
         proposed_version, diff, evidence_ids_json, risk_level, \
         policy_decision, status, created_at] \
         <- [[$pid, $wid, $mk, $cv, $pv, $diff, $evids, $rl, $pd, $status, $ca]] \
         :put behaviour_proposals { proposal_id => workspace_id, manifest_kind, \
         current_version, proposed_version, diff, evidence_ids_json, risk_level, \
         policy_decision, status, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert behaviour_proposals failed: {e}"))?;
    Ok(())
}

pub fn get_behaviour_proposal(
    db: &DbInstance,
    proposal_id: &str,
) -> Result<Option<BehaviourProposal>> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal_id));

    let result = db
        .run_script(
            "?[proposal_id, workspace_id, manifest_kind, current_version, \
             proposed_version, diff, evidence_ids_json, risk_level, \
             policy_decision, status, created_at] \
             := *behaviour_proposals{proposal_id, workspace_id, manifest_kind, \
             current_version, proposed_version, diff, evidence_ids_json, risk_level, \
             policy_decision, status, created_at}, proposal_id = $pid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get behaviour_proposal failed: {e}"))?;

    if result.rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(row_to_proposal(&result.rows[0])))
}

pub fn list_behaviour_proposals(
    db: &DbInstance,
    status_filter: Option<&str>,
) -> Result<Vec<BehaviourProposal>> {
    let result = if let Some(status) = status_filter {
        let mut params = BTreeMap::new();
        params.insert("status".into(), DataValue::from(status));
        db.run_script(
            "?[proposal_id, workspace_id, manifest_kind, current_version, \
             proposed_version, diff, evidence_ids_json, risk_level, \
             policy_decision, status, created_at] \
             := *behaviour_proposals{proposal_id, workspace_id, manifest_kind, \
             current_version, proposed_version, diff, evidence_ids_json, risk_level, \
             policy_decision, status, created_at}, status = $status",
            params,
            ScriptMutability::Immutable,
        )
    } else {
        db.run_script(
            "?[proposal_id, workspace_id, manifest_kind, current_version, \
             proposed_version, diff, evidence_ids_json, risk_level, \
             policy_decision, status, created_at] \
             := *behaviour_proposals{proposal_id, workspace_id, manifest_kind, \
             current_version, proposed_version, diff, evidence_ids_json, risk_level, \
             policy_decision, status, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
        )
    };
    let result = result.map_err(|e| anyhow::anyhow!("list behaviour_proposals failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .map(|row| row_to_proposal(row))
        .collect())
}

fn row_to_proposal(row: &[DataValue]) -> BehaviourProposal {
    BehaviourProposal {
        proposal_id: row[0].get_str().unwrap_or("").to_string(),
        workspace_id: row[1].get_str().unwrap_or("").to_string(),
        manifest_kind: BehaviourManifestKind::from_str(row[2].get_str().unwrap_or(""))
            .unwrap_or(BehaviourManifestKind::RetrievalProfile),
        current_version: row[3].get_str().unwrap_or("").to_string(),
        proposed_version: row[4].get_str().unwrap_or("").to_string(),
        diff: row[5].get_str().unwrap_or("").to_string(),
        evidence_ids: serde_json::from_str(row[6].get_str().unwrap_or("[]")).unwrap_or_default(),
        risk_level: RiskLevel::from_str(row[7].get_str().unwrap_or("Low")).unwrap_or(RiskLevel::Low),
        policy_decision: PolicyDecision::from_str(row[8].get_str().unwrap_or("PendingApproval"))
            .unwrap_or(PolicyDecision::PendingApproval),
        status: ProposalStatus::from_str(row[9].get_str().unwrap_or("Pending"))
            .unwrap_or(ProposalStatus::Pending),
        created_at: row[10].get_str().unwrap_or("").to_string(),
    }
}

/// Update proposal status (read-modify-write).
pub fn update_proposal_status(
    db: &DbInstance,
    proposal_id: &str,
    status: &ProposalStatus,
    policy_decision: &PolicyDecision,
) -> Result<()> {
    let mut proposal = get_behaviour_proposal(db, proposal_id)?
        .ok_or_else(|| anyhow::anyhow!("proposal not found: {proposal_id}"))?;
    proposal.status = status.clone();
    proposal.policy_decision = policy_decision.clone();
    insert_behaviour_proposal(db, &proposal)
}

// ── BehaviourManifestVersion ───────────────────────────────────────────────────

pub fn insert_manifest_version(
    db: &DbInstance,
    version: &BehaviourManifestVersion,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("vid".into(), DataValue::from(version.version_id.as_str()));
    params.insert("mk".into(), DataValue::from(version.manifest_kind.as_str()));
    params.insert("vn".into(), DataValue::from(version.version_number));
    params.insert(
        "content".into(),
        DataValue::from(version.content.to_string().as_str()),
    );
    params.insert(
        "diff".into(),
        DataValue::from(version.diff.as_str()),
    );
    params.insert(
        "pvid".into(),
        DataValue::from(version.parent_version_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "cbid".into(),
        DataValue::from(version.created_by_proposal_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "rt".into(),
        DataValue::from(version.is_rollback_target),
    );
    params.insert("ca".into(), DataValue::from(version.created_at.as_str()));

    db.run_script(
        "?[version_id, manifest_kind, version_number, content_json, diff, \
         parent_version_id, created_by_proposal_id, is_rollback_target, created_at] \
         <- [[$vid, $mk, $vn, $content, $diff, $pvid, $cbid, $rt, $ca]] \
         :put behaviour_manifest_versions { version_id => manifest_kind, \
         version_number, content_json, diff, parent_version_id, \
         created_by_proposal_id, is_rollback_target, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert behaviour_manifest_versions failed: {e}"))?;
    Ok(())
}

pub fn get_manifest_version(
    db: &DbInstance,
    version_id: &str,
) -> Result<Option<BehaviourManifestVersion>> {
    let mut params = BTreeMap::new();
    params.insert("vid".into(), DataValue::from(version_id));

    let result = db
        .run_script(
            "?[version_id, manifest_kind, version_number, content_json, diff, \
             parent_version_id, created_by_proposal_id, is_rollback_target, created_at] \
             := *behaviour_manifest_versions{version_id, manifest_kind, \
             version_number, content_json, diff, parent_version_id, \
             created_by_proposal_id, is_rollback_target, created_at}, version_id = $vid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get manifest_version failed: {e}"))?;

    if result.rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(row_to_manifest_version(&result.rows[0])))
}

pub fn get_latest_manifest_version(
    db: &DbInstance,
    manifest_kind: &str,
) -> Result<Option<BehaviourManifestVersion>> {
    // Get all versions for this kind, ordered by created_at descending
    let mut params = BTreeMap::new();
    params.insert("mk".into(), DataValue::from(manifest_kind));

    let result = db
        .run_script(
            "?[version_id, manifest_kind, version_number, content_json, diff, \
             parent_version_id, created_by_proposal_id, is_rollback_target, created_at] \
             := *behaviour_manifest_versions{version_id, manifest_kind, \
             version_number, content_json, diff, parent_version_id, \
             created_by_proposal_id, is_rollback_target, created_at}, manifest_kind = $mk",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get latest manifest_version failed: {e}"))?;

    if result.rows.is_empty() {
        return Ok(None);
    }

    // Find the most recent by created_at
    let mut versions: Vec<BehaviourManifestVersion> = result
        .rows
        .iter()
        .map(|row| row_to_manifest_version(row))
        .collect();
    versions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(Some(versions.into_iter().next().unwrap()))
}

pub fn list_manifest_version_history(
    db: &DbInstance,
    manifest_kind: &str,
) -> Result<Vec<BehaviourManifestVersion>> {
    let mut params = BTreeMap::new();
    params.insert("mk".into(), DataValue::from(manifest_kind));

    let result = db
        .run_script(
            "?[version_id, manifest_kind, version_number, content_json, diff, \
             parent_version_id, created_by_proposal_id, is_rollback_target, created_at] \
             := *behaviour_manifest_versions{version_id, manifest_kind, \
             version_number, content_json, diff, parent_version_id, \
             created_by_proposal_id, is_rollback_target, created_at}, manifest_kind = $mk",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list manifest version history failed: {e}"))?;

    let mut versions: Vec<BehaviourManifestVersion> = result
        .rows
        .iter()
        .map(|row| row_to_manifest_version(row))
        .collect();
    versions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(versions)
}

fn row_to_manifest_version(row: &[DataValue]) -> BehaviourManifestVersion {
    BehaviourManifestVersion {
        version_id: row[0].get_str().unwrap_or("").to_string(),
        manifest_kind: BehaviourManifestKind::from_str(row[1].get_str().unwrap_or(""))
            .unwrap_or(BehaviourManifestKind::RetrievalProfile),
        version_number: row[2].get_int().unwrap_or(1),
        content: {
            let s = row[3].get_str().unwrap_or("{}");
            serde_json::from_str(s).unwrap_or(serde_json::Value::Object(Default::default()))
        },
        diff: row[4].get_str().unwrap_or("").to_string(),
        parent_version_id: {
            let s = row[5].get_str().unwrap_or("");
            if s.is_empty() { None } else { Some(s.to_string()) }
        },
        created_by_proposal_id: {
            let s = row[6].get_str().unwrap_or("");
            if s.is_empty() { None } else { Some(s.to_string()) }
        },
        is_rollback_target: row[7].get_bool().unwrap_or(false),
        created_at: row[8].get_str().unwrap_or("").to_string(),
    }
}

// ── PolicyDecision rows ────────────────────────────────────────────────────────

pub fn insert_policy_decision(
    db: &DbInstance,
    decision_id: &str,
    proposal_id: &str,
    rule_name: &str,
    outcome: &PolicyDecision,
    reason: &str,
    evaluated_inputs: &serde_json::Value,
    created_at: &str,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("did".into(), DataValue::from(decision_id));
    params.insert("pid".into(), DataValue::from(proposal_id));
    params.insert("rn".into(), DataValue::from(rule_name));
    params.insert("outcome".into(), DataValue::from(outcome.as_str()));
    params.insert("reason".into(), DataValue::from(reason));
    params.insert("ei".into(), DataValue::from(evaluated_inputs.to_string().as_str()));
    params.insert("ca".into(), DataValue::from(created_at));

    db.run_script(
        "?[decision_id, proposal_id, rule_name, outcome, reason, \
         evaluated_inputs_json, created_at] \
         <- [[$did, $pid, $rn, $outcome, $reason, $ei, $ca]] \
         :put behaviour_policy_decisions { decision_id => proposal_id, \
         rule_name, outcome, reason, evaluated_inputs_json, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert policy_decision failed: {e}"))?;
    Ok(())
}

pub fn list_policy_decisions_for_proposal(
    db: &DbInstance,
    proposal_id: &str,
) -> Result<Vec<PolicyOutcome>> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal_id));

    let result = db
        .run_script(
            "?[decision_id, proposal_id, rule_name, outcome, reason, \
             evaluated_inputs_json, created_at] \
             := *behaviour_policy_decisions{decision_id, proposal_id, \
             rule_name, outcome, reason, evaluated_inputs_json, created_at}, \
             proposal_id = $pid",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list policy_decisions failed: {e}"))?;

    Ok(result
        .rows
        .iter()
        .map(|row| PolicyOutcome {
            rule_name: row[2].get_str().unwrap_or("").to_string(),
            evaluated: serde_json::from_str(row[5].get_str().unwrap_or("{}"))
                .unwrap_or(serde_json::Value::Object(Default::default())),
            outcome: PolicyDecision::from_str(row[3].get_str().unwrap_or("PendingApproval"))
                .unwrap_or(PolicyDecision::PendingApproval),
            reason: row[4].get_str().unwrap_or("").to_string(),
        })
        .collect())
}

// ── BehaviourApproval ─────────────────────────────────────────────────────────

pub fn insert_approval(db: &DbInstance, approval: &BehaviourApproval) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("aid".into(), DataValue::from(approval.approval_id.as_str()));
    params.insert("pid".into(), DataValue::from(approval.proposal_id.as_str()));
    params.insert("approver".into(), DataValue::from(approval.approver.as_str()));
    params.insert("approved".into(), DataValue::from(approval.approved));
    params.insert("comment".into(), DataValue::from(approval.comment.as_str()));
    params.insert("ca".into(), DataValue::from(approval.created_at.as_str()));

    db.run_script(
        "?[approval_id, proposal_id, approver, approved, comment, created_at] \
         <- [[$aid, $pid, $approver, $approved, $comment, $ca]] \
         :put behaviour_approvals { approval_id => proposal_id, approver, \
         approved, comment, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert approval failed: {e}"))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-store-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn test_learning_event_roundtrip() {
        let db = test_db();

        let event = LearningEvent {
            event_id: "lev-test-roundtrip".into(),
            workspace_id: "ws-roundtrip".into(),
            event_type: LearningEventType::GatePassed,
            source_artifact_id: "gate-sherlock".into(),
            outcome_artifact_id: Some("out-1".into()),
            signal: serde_json::json!({"passed": true, "score": 0.95}),
            confidence: 0.92,
            provenance_record_id: "prov-1".into(),
            created_at: "2026-05-03T00:00:00Z".into(),
        };

        insert_learning_event(&db, &event).unwrap();

        let retrieved = get_learning_event(&db, "lev-test-roundtrip")
            .unwrap()
            .expect("event must be retrievable");

        assert_eq!(retrieved.event_id, event.event_id);
        assert_eq!(retrieved.workspace_id, event.workspace_id);
        assert_eq!(retrieved.event_type, LearningEventType::GatePassed);
        assert_eq!(retrieved.source_artifact_id, "gate-sherlock");
        assert_eq!(retrieved.outcome_artifact_id, Some("out-1".into()));
        assert_eq!(retrieved.signal, serde_json::json!({"passed": true, "score": 0.95}));
        assert!((retrieved.confidence - 0.92).abs() < 0.001);
        assert_eq!(retrieved.provenance_record_id, "prov-1");
    }
}
