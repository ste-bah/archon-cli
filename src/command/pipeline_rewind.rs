use std::path::Path;

use anyhow::{Context, Result};
use archon_pipeline::audit::{PipelineBundleStore, rewind_bundle};

pub(crate) async fn handle_rewind(
    cwd: &Path,
    session_id: &str,
    to_agent: Option<&str>,
    to_ordinal: Option<usize>,
    keep_agents: Option<usize>,
    reason: &str,
) -> Result<()> {
    let store = PipelineBundleStore::new(cwd);
    let keep_count = resolve_keep_count(&store, session_id, to_agent, to_ordinal, keep_agents)?;
    let report = rewind_bundle(&store, session_id, keep_count, reason)?;
    println!("Session: {}", report.session_id);
    println!(
        "Rewound completed agents: {} -> {}",
        report.from_completed_agent_count, report.to_completed_agent_count
    );
    println!(
        "Quarantined agent records: {}",
        report.quarantined_agent_records
    );
    println!("Quarantine: {}", report.quarantine_dir);
    println!("Next: archon pipeline resume {session_id}");
    Ok(())
}

fn resolve_keep_count(
    store: &PipelineBundleStore,
    session_id: &str,
    to_agent: Option<&str>,
    to_ordinal: Option<usize>,
    keep_agents: Option<usize>,
) -> Result<usize> {
    if let Some(count) = keep_agents {
        return Ok(count);
    }
    if let Some(ordinal) = to_ordinal {
        return Ok(ordinal);
    }
    if let Some(agent_key) = to_agent {
        let records = store.list_agent_records(session_id)?;
        let record = records
            .iter()
            .find(|record| record.agent_key == agent_key)
            .with_context(|| format!("agent '{agent_key}' not found in audited bundle"))?;
        return Ok(record.ordinal);
    }
    anyhow::bail!("provide one of --to-agent, --to-ordinal, or --keep-agents")
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_pipeline::audit::PipelineBundleStore;
    use archon_pipeline::audit::types::AgentAuditRecord;
    use archon_pipeline::runner::{PipelineType, ToolAccessLevel};

    #[test]
    fn resolve_keep_count_uses_agent_ordinal() {
        let temp = tempfile::tempdir().unwrap();
        let store = PipelineBundleStore::new(temp.path());
        store
            .create("session-1", PipelineType::Research, "task")
            .unwrap();
        let record = AgentAuditRecord {
            ordinal: 7,
            agent_key: "target-agent".into(),
            display_name: "Target Agent".into(),
            phase: 1,
            requested_model: "sonnet".into(),
            critical: true,
            quality_threshold: 0.5,
            tool_access_level: ToolAccessLevel::ReadOnly,
            prompt_record_path: "prompts/007-target-agent.json".into(),
            prompt_hash: "p".into(),
            system_hash: "s".into(),
            tools_hash: "t".into(),
            output_path: "outputs/007-target-agent.txt".into(),
            output_hash: "o".into(),
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            duration_ms: 0,
            quality: None,
            tool_use_log: Vec::new(),
            attempts: Vec::new(),
            completed_at: chrono::Utc::now(),
        };
        store.write_agent("session-1", &record).unwrap();

        let count =
            resolve_keep_count(&store, "session-1", Some("target-agent"), None, None).unwrap();
        assert_eq!(count, 7);
    }
}
