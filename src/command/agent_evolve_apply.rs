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

    mark_previous_active_for_rollback(db, parent.as_ref(), activate)?;
    archon_learning::agent_profile_versions::insert_agent_profile_version(db, &record)?;
    archon_learning::agent_evolution_proposals::update_agent_evolution_proposal_status(
        db,
        proposal_id,
        "applied",
    )?;
    record_profile_learning_event(
        db,
        &proposal.agent_type,
        archon_learning::models::LearningEventType::ManifestApplied,
        &proposal.proposal_id,
        &record.version_id,
        serde_json::json!({
            "source": "agent_evolution_apply",
            "kind": proposal.kind,
            "activate": activate,
        }),
    )?;

    Ok(AppliedAgentProfile {
        version_id: record.version_id,
        active: record.is_active,
    })
}

pub(crate) fn rollback_profile(
    db: &DbInstance,
    agent_type: &str,
    target_version_id: &str,
    activate: bool,
) -> Result<AppliedAgentProfile> {
    let target =
        archon_learning::agent_profile_versions::get_agent_profile_version(db, target_version_id)?
            .ok_or_else(|| anyhow::anyhow!("profile version not found: {target_version_id}"))?;
    if target.agent_type != agent_type {
        anyhow::bail!(
            "profile version {} belongs to agent {}, not {}",
            target_version_id,
            target.agent_type,
            agent_type
        );
    }

    let parent =
        archon_learning::agent_profile_versions::get_active_agent_profile_version(db, agent_type)?;
    let mut version = archon_core::agents::evolution::AgentProfileVersion::new(
        agent_type.to_string(),
        next_version_number(db, agent_type)?,
        archon_core::agents::evolution::AgentProfileVersionSource::Rollback,
        target.profile_json.clone(),
    );
    if let Some(parent) = &parent {
        version = version.with_parent(parent.version_id.clone());
    }
    if activate {
        version = version.mark_active();
    }
    let record = profile_record_from_core(&version);

    mark_previous_active_for_rollback(db, parent.as_ref(), activate)?;
    archon_learning::agent_profile_versions::insert_agent_profile_version(db, &record)?;
    record_profile_learning_event(
        db,
        agent_type,
        archon_learning::models::LearningEventType::ManifestRolledBack,
        target_version_id,
        &record.version_id,
        serde_json::json!({
            "source": "agent_evolution_rollback",
            "target_version_id": target_version_id,
            "activate": activate,
        }),
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
        "overrides": profile_overrides_for_proposal(proposal),
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

fn profile_overrides_for_proposal(
    proposal: &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord,
) -> serde_json::Value {
    let note = proposal.diff.trim();
    match proposal.kind.as_str() {
        "prompt_profile" => serde_json::json!({
            "system_prompt_append": format!(
                "Governed evolution note: review the linked correction evidence before finalizing. {note}"
            )
        }),
        "quality_gate_profile" => serde_json::json!({
            "tool_guidance_append": format!(
                "Governed evolution note: check and remediate the repeated quality gate pattern before final output. {note}"
            )
        }),
        "tool_access_profile" => serde_json::json!({
            "tool_guidance_append": format!(
                "Governed permission note: use explicit approval or safer alternatives for repeated tool denials. Do not grant tools automatically. {note}"
            )
        }),
        _ => serde_json::json!({}),
    }
}

fn profile_record_from_core(
    version: &archon_core::agents::evolution::AgentProfileVersion,
) -> archon_learning::agent_profile_versions::AgentProfileVersionRecord {
    let mut record = archon_learning::agent_profile_versions::AgentProfileVersionRecord::new(
        version.version_id.clone(),
        version.agent_type.clone(),
        version.version_number,
        profile_source_str(version.source),
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

fn mark_previous_active_for_rollback(
    db: &DbInstance,
    previous_active: Option<&archon_learning::agent_profile_versions::AgentProfileVersionRecord>,
    activate: bool,
) -> Result<()> {
    if !activate {
        return Ok(());
    }
    if let Some(previous_active) = previous_active {
        let mut updated = previous_active.clone();
        updated.is_active = false;
        updated.is_rollback_target = true;
        archon_learning::agent_profile_versions::insert_agent_profile_version(db, &updated)?;
    }
    Ok(())
}

fn profile_source_str(
    source: archon_core::agents::evolution::AgentProfileVersionSource,
) -> &'static str {
    match source {
        archon_core::agents::evolution::AgentProfileVersionSource::FileDefinition => {
            "file_definition"
        }
        archon_core::agents::evolution::AgentProfileVersionSource::GovernedProposal => {
            "governed_proposal"
        }
        archon_core::agents::evolution::AgentProfileVersionSource::ManualOperator => {
            "manual_operator"
        }
        archon_core::agents::evolution::AgentProfileVersionSource::Rollback => "rollback",
    }
}

fn record_profile_learning_event(
    db: &DbInstance,
    agent_type: &str,
    event_type: archon_learning::models::LearningEventType,
    source_artifact_id: &str,
    outcome_artifact_id: &str,
    signal: serde_json::Value,
) -> Result<()> {
    let event = archon_learning::models::LearningEvent {
        event_id: format!("learning-{}", uuid::Uuid::new_v4()),
        workspace_id: format!("agent:{agent_type}"),
        event_type,
        source_artifact_id: source_artifact_id.to_string(),
        outcome_artifact_id: Some(outcome_artifact_id.to_string()),
        signal,
        confidence: 1.0,
        provenance_record_id: "agent_profile_versions".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    archon_learning::store::insert_learning_event(db, &event)
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
        let events =
            archon_learning::store::list_learning_events_by_type(&db, "ManifestApplied").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source_artifact_id, "agent-evo-prop-1");
        assert_eq!(
            events[0].outcome_artifact_id.as_deref(),
            Some(applied.version_id.as_str())
        );
    }

    #[test]
    fn prompt_profile_apply_payload_contains_safe_overlay() {
        let db = test_db();
        archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
            &db,
            &proposal("approved"),
        )
        .unwrap();

        let applied = apply_proposal(&db, "agent-evo-prop-1", true).unwrap();
        let version = archon_learning::agent_profile_versions::get_agent_profile_version(
            &db,
            &applied.version_id,
        )
        .unwrap()
        .unwrap();

        assert!(version.is_active);
        assert!(
            version.profile_json["overrides"]["system_prompt_append"]
                .as_str()
                .unwrap()
                .contains("Governed evolution note")
        );
        assert!(version.profile_json["overrides"].get("provider").is_none());
        assert!(
            version.profile_json["overrides"]
                .get("identity_spoof")
                .is_none()
        );
    }

    #[test]
    fn tool_access_apply_payload_never_grants_permissions() {
        let proposal =
            archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
                "agent-evo-perm-1",
                "reviewer",
                "agentv-1",
                "agentv-2",
                "tool_access_profile",
                "2026-05-08T12:00:00Z",
            )
            .with_diff("+ review repeated denied tool `Bash`; do not grant automatically")
            .with_permission_impact()
            .with_risk("high", "pending_approval")
            .with_status("approved");

        let version = build_profile_version(&proposal, None, 1, true);
        let overrides = &version.profile_json["overrides"];

        assert!(
            overrides["tool_guidance_append"]
                .as_str()
                .unwrap()
                .contains("Do not grant tools automatically")
        );
        assert!(overrides.get("permission_mode").is_none());
        assert!(overrides.get("allowed_tools").is_none());
        assert!(overrides.get("sandbox_backend").is_none());
    }

    #[test]
    fn rollback_creates_profile_version_from_target() {
        let db = test_db();
        let target = archon_learning::agent_profile_versions::AgentProfileVersionRecord::new(
            "agent-profile-1",
            "reviewer",
            1,
            "file_definition",
            "2026-05-08T12:00:00Z",
        )
        .with_profile_json(serde_json::json!({"model": "claude-sonnet-4-6"}))
        .mark_active();
        archon_learning::agent_profile_versions::insert_agent_profile_version(&db, &target)
            .unwrap();

        let applied = rollback_profile(&db, "reviewer", "agent-profile-1", true).unwrap();
        let rollback = archon_learning::agent_profile_versions::get_agent_profile_version(
            &db,
            &applied.version_id,
        )
        .unwrap()
        .unwrap();
        let old_active = archon_learning::agent_profile_versions::get_agent_profile_version(
            &db,
            "agent-profile-1",
        )
        .unwrap()
        .unwrap();

        assert!(rollback.is_active);
        assert_eq!(rollback.source, "rollback");
        assert_eq!(rollback.profile_json["model"], "claude-sonnet-4-6");
        assert!(!old_active.is_active);
        assert!(old_active.is_rollback_target);
        let events =
            archon_learning::store::list_learning_events_by_type(&db, "ManifestRolledBack")
                .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source_artifact_id, "agent-profile-1");
        assert_eq!(
            events[0].outcome_artifact_id.as_deref(),
            Some(applied.version_id.as_str())
        );
    }
}
