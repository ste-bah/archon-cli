use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentEvolutionProposalRecord {
    pub proposal_id: String,
    pub agent_type: String,
    pub current_version: String,
    pub proposed_version: String,
    pub kind: String,
    pub diff: String,
    pub evidence_ids: Vec<String>,
    pub risk_level: String,
    pub policy_decision: String,
    pub status: String,
    pub expected_impact: String,
    pub rollback_target_version: String,
    pub affects_provider_identity: bool,
    pub affects_permissions: bool,
    pub created_at: String,
}

impl AgentEvolutionProposalRecord {
    pub fn new(
        proposal_id: impl Into<String>,
        agent_type: impl Into<String>,
        current_version: impl Into<String>,
        proposed_version: impl Into<String>,
        kind: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        let current_version = current_version.into();
        Self {
            proposal_id: proposal_id.into(),
            agent_type: agent_type.into(),
            current_version: current_version.clone(),
            proposed_version: proposed_version.into(),
            kind: kind.into(),
            diff: String::new(),
            evidence_ids: Vec::new(),
            risk_level: "low".to_string(),
            policy_decision: "eligible_for_auto_apply".to_string(),
            status: "pending".to_string(),
            expected_impact: String::new(),
            rollback_target_version: current_version,
            affects_provider_identity: false,
            affects_permissions: false,
            created_at: created_at.into(),
        }
    }

    pub fn with_diff(mut self, diff: impl Into<String>) -> Self {
        self.diff = diff.into();
        self
    }

    pub fn with_evidence(mut self, evidence_id: impl Into<String>) -> Self {
        let evidence_id = evidence_id.into();
        if !self.evidence_ids.contains(&evidence_id) {
            self.evidence_ids.push(evidence_id);
        }
        self
    }

    pub fn with_risk(
        mut self,
        risk_level: impl Into<String>,
        policy_decision: impl Into<String>,
    ) -> Self {
        self.risk_level = risk_level.into();
        self.policy_decision = policy_decision.into();
        self
    }

    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = status.into();
        self
    }

    pub fn with_expected_impact(mut self, expected_impact: impl Into<String>) -> Self {
        self.expected_impact = expected_impact.into();
        self
    }

    pub fn with_provider_identity_impact(mut self) -> Self {
        self.affects_provider_identity = true;
        self
    }

    pub fn with_permission_impact(mut self) -> Self {
        self.affects_permissions = true;
        self
    }
}

pub fn insert_agent_evolution_proposal(
    db: &DbInstance,
    proposal: &AgentEvolutionProposalRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal.proposal_id.as_str()));
    params.insert(
        "agent".into(),
        DataValue::from(proposal.agent_type.as_str()),
    );
    params.insert(
        "current".into(),
        DataValue::from(proposal.current_version.as_str()),
    );
    params.insert(
        "proposed".into(),
        DataValue::from(proposal.proposed_version.as_str()),
    );
    params.insert("kind".into(), DataValue::from(proposal.kind.as_str()));
    params.insert("diff".into(), DataValue::from(proposal.diff.as_str()));
    params.insert(
        "evidence".into(),
        DataValue::from(serde_json::to_string(&proposal.evidence_ids)?.as_str()),
    );
    params.insert("risk".into(), DataValue::from(proposal.risk_level.as_str()));
    params.insert(
        "decision".into(),
        DataValue::from(proposal.policy_decision.as_str()),
    );
    params.insert("status".into(), DataValue::from(proposal.status.as_str()));
    params.insert(
        "impact".into(),
        DataValue::from(proposal.expected_impact.as_str()),
    );
    params.insert(
        "rollback".into(),
        DataValue::from(proposal.rollback_target_version.as_str()),
    );
    params.insert(
        "identity".into(),
        DataValue::from(proposal.affects_provider_identity),
    );
    params.insert(
        "permissions".into(),
        DataValue::from(proposal.affects_permissions),
    );
    params.insert(
        "created".into(),
        DataValue::from(proposal.created_at.as_str()),
    );

    db.run_script(proposal_put_script(), params, ScriptMutability::Mutable)
        .map_err(|e| anyhow::anyhow!("insert agent_evolution_proposals failed: {e}"))?;
    Ok(())
}

pub fn get_agent_evolution_proposal(
    db: &DbInstance,
    proposal_id: &str,
) -> Result<Option<AgentEvolutionProposalRecord>> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal_id));
    let result = db
        .run_script(
            proposal_query("proposal_id = $pid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get agent_evolution_proposal failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_proposal(row)))
}

pub fn list_agent_evolution_proposals(
    db: &DbInstance,
    status: Option<&str>,
) -> Result<Vec<AgentEvolutionProposalRecord>> {
    let result = if let Some(status) = status {
        let mut params = BTreeMap::new();
        params.insert("status".into(), DataValue::from(status));
        db.run_script(
            proposal_query("status = $status"),
            params,
            ScriptMutability::Immutable,
        )
    } else {
        db.run_script(
            proposal_query("all"),
            Default::default(),
            ScriptMutability::Immutable,
        )
    }
    .map_err(|e| anyhow::anyhow!("list agent_evolution_proposals failed: {e}"))?;

    let mut proposals: Vec<_> = result.rows.iter().map(|row| row_to_proposal(row)).collect();
    proposals.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(proposals)
}

pub fn update_agent_evolution_proposal_status(
    db: &DbInstance,
    proposal_id: &str,
    status: &str,
) -> Result<AgentEvolutionProposalRecord> {
    let mut proposal = get_agent_evolution_proposal(db, proposal_id)?
        .ok_or_else(|| anyhow::anyhow!("agent evolution proposal not found: {proposal_id}"))?;
    proposal.status = status.to_string();
    insert_agent_evolution_proposal(db, &proposal)?;
    Ok(proposal)
}

fn proposal_put_script() -> &'static str {
    "?[proposal_id, agent_type, current_version, proposed_version, kind, \
     diff, evidence_ids_json, risk_level, policy_decision, status, \
     expected_impact, rollback_target_version, affects_provider_identity, \
     affects_permissions, created_at] <- [[$pid, $agent, $current, \
     $proposed, $kind, $diff, $evidence, $risk, $decision, $status, \
     $impact, $rollback, $identity, $permissions, $created]] \
     :put agent_evolution_proposals { proposal_id => agent_type, \
     current_version, proposed_version, kind, diff, evidence_ids_json, \
     risk_level, policy_decision, status, expected_impact, \
     rollback_target_version, affects_provider_identity, affects_permissions, \
     created_at }"
}

fn proposal_query(predicate: &'static str) -> &'static str {
    match predicate {
        "proposal_id = $pid" => {
            "?[proposal_id, agent_type, current_version, proposed_version, kind, \
             diff, evidence_ids_json, risk_level, policy_decision, status, \
             expected_impact, rollback_target_version, affects_provider_identity, \
             affects_permissions, created_at] := *agent_evolution_proposals{ \
             proposal_id, agent_type, current_version, proposed_version, kind, \
             diff, evidence_ids_json, risk_level, policy_decision, status, \
             expected_impact, rollback_target_version, affects_provider_identity, \
             affects_permissions, created_at }, proposal_id = $pid"
        }
        "status = $status" => {
            "?[proposal_id, agent_type, current_version, proposed_version, kind, \
             diff, evidence_ids_json, risk_level, policy_decision, status, \
             expected_impact, rollback_target_version, affects_provider_identity, \
             affects_permissions, created_at] := *agent_evolution_proposals{ \
             proposal_id, agent_type, current_version, proposed_version, kind, \
             diff, evidence_ids_json, risk_level, policy_decision, status, \
             expected_impact, rollback_target_version, affects_provider_identity, \
             affects_permissions, created_at }, status = $status"
        }
        _ => {
            "?[proposal_id, agent_type, current_version, proposed_version, kind, \
             diff, evidence_ids_json, risk_level, policy_decision, status, \
             expected_impact, rollback_target_version, affects_provider_identity, \
             affects_permissions, created_at] := *agent_evolution_proposals{ \
             proposal_id, agent_type, current_version, proposed_version, kind, \
             diff, evidence_ids_json, risk_level, policy_decision, status, \
             expected_impact, rollback_target_version, affects_provider_identity, \
             affects_permissions, created_at }"
        }
    }
}

fn row_to_proposal(row: &[DataValue]) -> AgentEvolutionProposalRecord {
    AgentEvolutionProposalRecord {
        proposal_id: str_col(row, 0).to_string(),
        agent_type: str_col(row, 1).to_string(),
        current_version: str_col(row, 2).to_string(),
        proposed_version: str_col(row, 3).to_string(),
        kind: str_col(row, 4).to_string(),
        diff: str_col(row, 5).to_string(),
        evidence_ids: serde_json::from_str(str_col(row, 6)).unwrap_or_default(),
        risk_level: str_col(row, 7).to_string(),
        policy_decision: str_col(row, 8).to_string(),
        status: str_col(row, 9).to_string(),
        expected_impact: str_col(row, 10).to_string(),
        rollback_target_version: str_col(row, 11).to_string(),
        affects_provider_identity: row[12].get_bool().unwrap_or(false),
        affects_permissions: row[13].get_bool().unwrap_or(false),
        created_at: str_col(row, 14).to_string(),
    }
}

fn str_col(row: &[DataValue], index: usize) -> &str {
    row[index].get_str().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-agent-evolution-proposals-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn agent_evolution_proposal_roundtrips() {
        let db = test_db();
        let proposal = AgentEvolutionProposalRecord::new(
            "agent-evo-prop-1",
            "reviewer",
            "agentv-1",
            "agentv-2",
            "prompt_profile",
            "2026-05-08T12:00:00Z",
        )
        .with_diff("+ require provenance")
        .with_evidence("ledger-1")
        .with_risk("high", "pending_approval")
        .with_expected_impact("Reduce unsupported claims")
        .with_provider_identity_impact();

        insert_agent_evolution_proposal(&db, &proposal).unwrap();
        let restored = get_agent_evolution_proposal(&db, "agent-evo-prop-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.agent_type, "reviewer");
        assert_eq!(restored.kind, "prompt_profile");
        assert_eq!(restored.evidence_ids, vec!["ledger-1"]);
        assert!(restored.affects_provider_identity);
    }

    #[test]
    fn agent_evolution_proposals_filter_by_status() {
        let db = test_db();
        insert_agent_evolution_proposal(
            &db,
            &AgentEvolutionProposalRecord::new(
                "agent-evo-prop-1",
                "planner",
                "agentv-1",
                "agentv-2",
                "model_profile",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();
        insert_agent_evolution_proposal(
            &db,
            &AgentEvolutionProposalRecord::new(
                "agent-evo-prop-2",
                "planner",
                "agentv-2",
                "agentv-3",
                "tool_access_profile",
                "2026-05-08T12:01:00Z",
            )
            .with_status("rejected")
            .with_permission_impact(),
        )
        .unwrap();

        let all = list_agent_evolution_proposals(&db, None).unwrap();
        let rejected = list_agent_evolution_proposals(&db, Some("rejected")).unwrap();

        assert_eq!(all.len(), 2);
        assert_eq!(all[0].proposal_id, "agent-evo-prop-2");
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0].affects_permissions);
    }

    #[test]
    fn agent_evolution_proposal_status_updates() {
        let db = test_db();
        insert_agent_evolution_proposal(
            &db,
            &AgentEvolutionProposalRecord::new(
                "agent-evo-prop-1",
                "planner",
                "agentv-1",
                "agentv-2",
                "model_profile",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();

        let updated =
            update_agent_evolution_proposal_status(&db, "agent-evo-prop-1", "approved").unwrap();
        let restored = get_agent_evolution_proposal(&db, "agent-evo-prop-1")
            .unwrap()
            .unwrap();

        assert_eq!(updated.status, "approved");
        assert_eq!(restored.status, "approved");
        assert_eq!(restored.kind, "model_profile");
    }
}
