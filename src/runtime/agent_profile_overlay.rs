//! Runtime bridge from Cozo-backed profile versions into agent definitions.

use anyhow::Result;
use cozo::DbInstance;

pub(crate) fn apply_active_profile_overlay_if_enabled(
    config: &archon_core::config::ArchonConfig,
    agent: &mut archon_core::agents::CustomAgentDefinition,
) -> Result<Option<archon_core::agents::evolution::AgentProfileOverlayReport>> {
    if !config
        .learning
        .agent_evolution
        .active_profile_overlay_enabled
    {
        return Ok(None);
    }

    let db_path = learning_db_path()?;
    let db = open_learning_db(&db_path)?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    hydrate_agent_meta_from_ledger(&db, agent)?;
    let Some(active) = archon_learning::agent_profile_versions::get_active_agent_profile_version(
        &db,
        &agent.agent_type,
    )?
    else {
        tracing::debug!(
            agent = %agent.agent_type,
            "active profile overlay enabled but no active profile exists"
        );
        return Ok(None);
    };

    let report = apply_profile_version_record(agent, &active);
    tracing::info!(
        agent = %agent.agent_type,
        profile_version = %report.version_id,
        applied = report.applied_fields.len(),
        ignored = report.ignored_fields.len(),
        "applied governed active profile overlay"
    );
    Ok(Some(report))
}

fn apply_profile_version_record(
    agent: &mut archon_core::agents::CustomAgentDefinition,
    active: &archon_learning::agent_profile_versions::AgentProfileVersionRecord,
) -> archon_core::agents::evolution::AgentProfileOverlayReport {
    archon_core::agents::evolution::apply_agent_profile_overlay(
        agent,
        active.version_id.clone(),
        &active.profile_json,
    )
}

fn hydrate_agent_meta_from_ledger(
    db: &DbInstance,
    agent: &mut archon_core::agents::CustomAgentDefinition,
) -> Result<()> {
    let records = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
        db,
        &agent.agent_type,
    )?;
    if records.is_empty() {
        return Ok(());
    }

    agent.meta.invocation_count = records.len() as u64;
    if let Some(applied_rate) = average(records.iter().filter_map(|record| {
        record
            .applied_rate
            .or_else(|| record.user_accepted.map(bool_score))
    })) {
        agent.meta.quality.applied_rate = applied_rate;
    }
    if let Some(completion_rate) = average(records.iter().map(|record| {
        record.completion_rate.unwrap_or_else(|| {
            if record.completion_status == "succeeded" {
                1.0
            } else {
                0.0
            }
        })
    })) {
        agent.meta.quality.completion_rate = completion_rate;
    }
    agent.meta.updated_at = chrono::Utc::now();
    Ok(())
}

fn average(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut count = 0_u64;
    let mut total = 0.0;
    for value in values {
        count += 1;
        total += value.clamp(0.0, 1.0);
    }
    (count > 0).then_some(total / count as f64)
}

fn bool_score(value: bool) -> f64 {
    if value { 1.0 } else { 0.0 }
}

fn learning_db_path() -> Result<std::path::PathBuf> {
    Ok(crate::command::store_paths::evidence_db_path(&[
        "ARCHON_LEARNING_DB_PATH",
    ]))
}

fn open_learning_db(path: &std::path::Path) -> Result<DbInstance> {
    let path_str = path.to_string_lossy().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    DbInstance::new("sqlite", &path_str, "").map_err(|e| anyhow::anyhow!("open learning db: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> archon_core::agents::CustomAgentDefinition {
        archon_core::agents::CustomAgentDefinition {
            agent_type: "reviewer".to_string(),
            system_prompt: "Review carefully.".to_string(),
            ..archon_core::agents::CustomAgentDefinition::default()
        }
    }

    #[test]
    fn disabled_config_does_not_apply_overlay() {
        let mut config = archon_core::config::ArchonConfig::default();
        config
            .learning
            .agent_evolution
            .active_profile_overlay_enabled = false;
        let mut agent = agent();

        let report = apply_active_profile_overlay_if_enabled(&config, &mut agent).unwrap();

        assert!(report.is_none());
        assert_eq!(agent.system_prompt, "Review carefully.");
    }

    #[test]
    fn active_record_applies_without_provider_identity_fields() {
        let mut agent = agent();
        let record = archon_learning::agent_profile_versions::AgentProfileVersionRecord::new(
            "agent-profile-2",
            "reviewer",
            2,
            "governed_proposal",
            "2026-05-08T12:00:00Z",
        )
        .with_profile_json(serde_json::json!({
            "overrides": {
                "system_prompt_append": "Check provenance first.",
                "provider": "openai-codex",
                "identity_spoof": false
            }
        }))
        .mark_active();

        let report = apply_profile_version_record(&mut agent, &record);

        assert_eq!(
            agent.system_prompt,
            "Review carefully.\n\nCheck provenance first."
        );
        assert_eq!(report.applied_fields, vec!["system_prompt_append"]);
        assert!(report.ignored_fields.contains(&"provider".to_string()));
        assert!(
            report
                .ignored_fields
                .contains(&"identity_spoof".to_string())
        );
    }

    #[test]
    fn ledger_hydration_updates_runtime_meta_without_files() {
        let path = format!("/tmp/test-agent-overlay-meta-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        archon_learning::agent_evolution_ledger::insert_agent_performance_ledger_record(
            &db,
            &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
                "ledger-1",
                "reviewer",
                "succeeded",
                "2026-05-08T12:00:00Z",
            )
            .with_scores(Some(0.9), None)
            .with_user_feedback(Some(true), None),
        )
        .unwrap();
        archon_learning::agent_evolution_ledger::insert_agent_performance_ledger_record(
            &db,
            &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
                "ledger-2",
                "reviewer",
                "failed",
                "2026-05-08T12:01:00Z",
            )
            .with_user_feedback(Some(false), Some(true)),
        )
        .unwrap();
        let mut agent = agent();

        hydrate_agent_meta_from_ledger(&db, &mut agent).unwrap();

        assert_eq!(agent.meta.invocation_count, 2);
        assert!((agent.meta.quality.applied_rate - 0.5).abs() < f64::EPSILON);
        assert!((agent.meta.quality.completion_rate - 0.5).abs() < f64::EPSILON);
    }
}
