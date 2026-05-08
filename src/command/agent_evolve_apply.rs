//! Apply approved agent evolution proposals into Cozo profile versions.

use anyhow::Result;
use cozo::DbInstance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppliedAgentProfile {
    pub(crate) version_id: String,
    pub(crate) active: bool,
}

pub(crate) fn apply_proposal(
    db: &DbInstance,
    proposal_id: &str,
    activate: bool,
) -> Result<AppliedAgentProfile> {
    let proposal =
        archon_learning::agent_evolution_proposals::get_agent_evolution_proposal(db, proposal_id)?
            .ok_or_else(|| anyhow::anyhow!("agent evolution proposal not found: {proposal_id}"))?;
    ensure_can_apply(&proposal)?;

    let parent = archon_learning::agent_profile_versions::get_active_agent_profile_version(
        db,
        &proposal.agent_type,
    )?;
    let version_number = next_version_number(db, &proposal.agent_type)?;
    let core_version = build_profile_version(&proposal, parent.as_ref(), version_number, activate);
    let record = profile_record_from_core(&core_version);

    archon_learning::agent_profile_versions::insert_agent_profile_version(db, &record)?;
    archon_learning::agent_evolution_proposals::update_agent_evolution_proposal_status(
        db,
        proposal_id,
        "applied",
    )?;

    Ok(AppliedAgentProfile {
        version_id: record.version_id,
        active: record.is_active,
    })
}

fn ensure_can_apply(
    proposal: &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord,
) -> Result<()> {
    match proposal.status.as_str() {
        "rejected" => anyhow::bail!("proposal {} is rejected", proposal.proposal_id),
        "applied" => anyhow::bail!("proposal {} is already applied", proposal.proposal_id),
        _ => {}
    }

    let requires_approval = matches!(proposal.risk_level.as_str(), "high" | "critical")
        || proposal.affects_permissions
        || proposal.affects_provider_identity;
    if requires_approval && proposal.status != "approved" {
        anyhow::bail!(
            "proposal {} requires approval before apply",
            proposal.proposal_id
        );
    }
    Ok(())
}

fn next_version_number(db: &DbInstance, agent_type: &str) -> Result<i64> {
    let versions =
        archon_learning::agent_profile_versions::list_agent_profile_versions(db, agent_type)?;
    Ok(versions
        .iter()
        .map(|version| version.version_number)
        .max()
        .unwrap_or(0)
        + 1)
}

fn build_profile_version(
    proposal: &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord,
    parent: Option<&archon_learning::agent_profile_versions::AgentProfileVersionRecord>,
    version_number: i64,
    activate: bool,
) -> archon_core::agents::evolution::AgentProfileVersion {
    let profile_json = serde_json::json!({
        "proposal_id": proposal.proposal_id,
        "proposed_version": proposal.proposed_version,
        "kind": proposal.kind,
        "diff": proposal.diff,
        "expected_impact": proposal.expected_impact,
        "evidence_ids": proposal.evidence_ids,
        "activate_requested": activate,
    });
    let mut version = archon_core::agents::evolution::AgentProfileVersion::new(
        proposal.agent_type.clone(),
        version_number,
        archon_core::agents::evolution::AgentProfileVersionSource::GovernedProposal,
        profile_json,
    )
    .with_proposal(proposal.proposal_id.clone());
    if let Some(parent) = parent {
        version = version.with_parent(parent.version_id.clone());
    }
    if activate {
        version = version.mark_active();
    }
    version
}

fn profile_record_from_core(
    version: &archon_core::agents::evolution::AgentProfileVersion,
) -> archon_learning::agent_profile_versions::AgentProfileVersionRecord {
    let mut record = archon_learning::agent_profile_versions::AgentProfileVersionRecord::new(
        version.version_id.clone(),
        version.agent_type.clone(),
        version.version_number,
        "governed_proposal",
        version.created_at.to_rfc3339(),
    )
    .with_profile_json(version.profile_json.clone())
    .with_hashes(
        version.prompt_hash.clone(),
        version.tools_hash.clone(),
        version.model_hash.clone(),
        version.memory_hash.clone(),
    );
    if let Some(parent_version_id) = &version.parent_version_id {
        record = record.with_parent(parent_version_id.clone());
    }
    if let Some(proposal_id) = &version.created_by_proposal_id {
        record = record.with_proposal(proposal_id.clone());
    }
    if version.is_active {
        record = record.mark_active();
    }
    if version.is_rollback_target {
        record = record.mark_rollback_target();
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-agent-evolve-apply-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    fn proposal(
        status: &str,
    ) -> archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord {
        archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
            "agent-evo-prop-1",
            "reviewer",
            "agentv-1",
            "agentv-2",
            "prompt_profile",
            "2026-05-08T12:00:00Z",
        )
        .with_diff("+ require provenance")
        .with_risk("high", "pending_approval")
        .with_status(status)
    }

    #[test]
    fn high_risk_proposal_requires_approval_before_apply() {
        let db = test_db();
        archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
            &db,
            &proposal("pending"),
        )
        .unwrap();

        let err = apply_proposal(&db, "agent-evo-prop-1", false).unwrap_err();

        assert!(err.to_string().contains("requires approval"));
    }

    #[test]
    fn approved_proposal_creates_profile_version_without_activation_by_default() {
        let db = test_db();
        archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
            &db,
            &proposal("approved"),
        )
        .unwrap();

        let applied = apply_proposal(&db, "agent-evo-prop-1", false).unwrap();
        let version = archon_learning::agent_profile_versions::get_agent_profile_version(
            &db,
            &applied.version_id,
        )
        .unwrap()
        .unwrap();
        let proposal = archon_learning::agent_evolution_proposals::get_agent_evolution_proposal(
            &db,
            "agent-evo-prop-1",
        )
        .unwrap()
        .unwrap();

        assert!(!applied.active);
        assert!(!version.is_active);
        assert_eq!(
            version.created_by_proposal_id.as_deref(),
            Some("agent-evo-prop-1")
        );
        assert_eq!(proposal.status, "applied");
    }
}
