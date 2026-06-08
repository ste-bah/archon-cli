//! Cozo-backed permission audit CLI surfaces.

use std::collections::BTreeMap;

use anyhow::Result;
use archon_learning::permission_runtime_events::{
    PermissionRuntimeEventRecord, list_permission_runtime_events,
    list_permission_runtime_events_by_decision,
};
use cozo::DbInstance;
use serde::Serialize;

use crate::cli_args::PermissionsAction;

#[path = "permissions_cli_diff.rs"]
mod permissions_cli_diff;

pub(crate) fn handle_permissions_command(action: &PermissionsAction) -> Result<()> {
    match action {
        PermissionsAction::Audit { json } => print!("{}", render_permissions_audit(*json)?),
        PermissionsAction::Denials { agent, limit, json } => print!(
            "{}",
            render_permission_denials(agent.as_deref(), *limit, *json)?
        ),
        PermissionsAction::Diff {
            agent,
            from_version_id,
            to_version_id,
            json,
        } => print!(
            "{}",
            render_permission_profile_diff(agent, from_version_id, to_version_id, *json)?
        ),
    }
    Ok(())
}

fn render_permissions_audit(json: bool) -> Result<String> {
    let db = open_learning_db()?;
    render_permissions_audit_from_db(&db, json)
}

fn render_permission_denials(
    agent_filter: Option<&str>,
    limit: usize,
    json: bool,
) -> Result<String> {
    let db = open_learning_db()?;
    render_permission_denials_from_db(&db, agent_filter, limit, json)
}

fn render_permission_profile_diff(
    agent_type: &str,
    from_version_id: &str,
    to_version_id: &str,
    json: bool,
) -> Result<String> {
    let db = open_learning_db()?;
    permissions_cli_diff::render_permission_profile_diff_from_db(
        &db,
        agent_type,
        from_version_id,
        to_version_id,
        json,
    )
}

fn render_permissions_audit_from_db(db: &DbInstance, json: bool) -> Result<String> {
    let events = list_permission_runtime_events(db)?;
    let audit = PermissionAudit::from_events(&events);
    if json {
        return Ok(format!("{}\n", serde_json::to_string_pretty(&audit)?));
    }

    let mut out = String::from("Permission audit (Cozo)\n\n");
    out.push_str(&format!("Events: {}\n", audit.total_events));
    out.push_str(&format!(
        "Decisions: {}\n",
        format_counts(&audit.by_decision)
    ));
    out.push_str(&format!("Denied: {}\n", audit.denied_count));
    out.push_str(&format!("Agents: {}\n", format_counts(&audit.by_agent)));
    out.push_str(&format!("Tools: {}\n", format_counts(&audit.by_tool)));
    out.push_str(&format!("Reasons: {}\n", format_counts(&audit.by_reason)));
    if !audit.recent_denials.is_empty() {
        out.push_str("\nRecent denials:\n");
        for item in &audit.recent_denials {
            out.push_str(&format!(
                "- {} {} {} {}\n",
                item.created_at, item.agent_type, item.tool_name, item.reason_code
            ));
        }
    }
    Ok(out)
}

fn render_permission_denials_from_db(
    db: &DbInstance,
    agent_filter: Option<&str>,
    limit: usize,
    json: bool,
) -> Result<String> {
    let denials = filtered_denials(db, agent_filter, limit)?;
    if json {
        return Ok(format!("{}\n", serde_json::to_string_pretty(&denials)?));
    }

    let mut out = String::from("Permission denials (Cozo)\n\n");
    if denials.is_empty() {
        out.push_str("No permission denials found.\n");
        return Ok(out);
    }
    out.push_str("created_at           agent              tool          mode        reason\n");
    out.push_str("--------------------------------------------------------------------------\n");
    for denial in denials {
        out.push_str(&format!(
            "{:<20} {:<18} {:<13} {:<11} {}\n",
            denial.created_at,
            denial.agent_type.as_deref().unwrap_or("-"),
            denial.tool_name,
            denial.permission_mode,
            denial.reason_code.as_deref().unwrap_or("permission_denied"),
        ));
    }
    Ok(out)
}

fn filtered_denials(
    db: &DbInstance,
    agent_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<PermissionRuntimeEventRecord>> {
    let mut denials = list_permission_runtime_events_by_decision(db, "denied")?;
    if let Some(agent) = agent_filter {
        denials.retain(|event| event.agent_type.as_deref() == Some(agent));
    }
    denials.truncate(limit);
    Ok(denials)
}

#[derive(Debug, Serialize, PartialEq)]
struct PermissionAudit {
    total_events: usize,
    denied_count: usize,
    by_decision: BTreeMap<String, usize>,
    by_agent: BTreeMap<String, usize>,
    by_tool: BTreeMap<String, usize>,
    by_reason: BTreeMap<String, usize>,
    recent_denials: Vec<PermissionDenialSummary>,
}

impl PermissionAudit {
    fn from_events(events: &[PermissionRuntimeEventRecord]) -> Self {
        let mut by_decision = BTreeMap::new();
        let mut by_agent = BTreeMap::new();
        let mut by_tool = BTreeMap::new();
        let mut by_reason = BTreeMap::new();
        for event in events {
            count(&mut by_decision, &event.decision);
            count(&mut by_tool, &event.tool_name);
            count(&mut by_agent, event.agent_type.as_deref().unwrap_or("-"));
            count(
                &mut by_reason,
                event.reason_code.as_deref().unwrap_or("unspecified"),
            );
        }
        let recent_denials = events
            .iter()
            .filter(|event| event.decision == "denied")
            .take(5)
            .map(PermissionDenialSummary::from)
            .collect::<Vec<_>>();
        Self {
            total_events: events.len(),
            denied_count: events
                .iter()
                .filter(|event| event.decision == "denied")
                .count(),
            by_decision,
            by_agent,
            by_tool,
            by_reason,
            recent_denials,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct PermissionDenialSummary {
    event_id: String,
    agent_type: String,
    tool_name: String,
    permission_mode: String,
    reason_code: String,
    created_at: String,
}

impl From<&PermissionRuntimeEventRecord> for PermissionDenialSummary {
    fn from(event: &PermissionRuntimeEventRecord) -> Self {
        Self {
            event_id: event.event_id.clone(),
            agent_type: event.agent_type.clone().unwrap_or_else(|| "-".to_string()),
            tool_name: event.tool_name.clone(),
            permission_mode: event.permission_mode.clone(),
            reason_code: event
                .reason_code
                .clone()
                .unwrap_or_else(|| "permission_denied".to_string()),
            created_at: event.created_at.clone(),
        }
    }
}

fn count(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.to_string()).or_default() += 1;
}

fn format_counts(counts: &BTreeMap<String, usize>) -> String {
    if counts.is_empty() {
        return "-".to_string();
    }
    counts
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn open_learning_db() -> Result<DbInstance> {
    let db = crate::command::store_paths::open_learning_db("learning")?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_learning::permission_runtime_events::insert_permission_runtime_event;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-permissions-cli-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn audit_counts_denials_without_raw_payload() {
        let db = test_db();
        insert_permission_runtime_event(
            &db,
            &PermissionRuntimeEventRecord::new(
                "permission-1",
                "Bash",
                "ask",
                "denied",
                "2026-05-08T12:00:00Z",
            )
            .with_run_context(Some("session-1".into()), Some("reviewer".into()))
            .with_policy_context(Some("deny_rule".into()), None, None)
            .with_raw_redacted_json(serde_json::json!({"payload": "redacted"})),
        )
        .unwrap();

        let body = render_permissions_audit_from_db(&db, false).unwrap();

        assert!(body.contains("Permission audit"));
        assert!(body.contains("denied:1"));
        assert!(body.contains("reviewer:1"));
        assert!(!body.contains("payload"));
    }

    #[test]
    fn denials_filter_by_agent_and_limit() {
        let db = test_db();
        for (idx, agent) in ["reviewer", "planner"].into_iter().enumerate() {
            insert_permission_runtime_event(
                &db,
                &PermissionRuntimeEventRecord::new(
                    format!("permission-{idx}"),
                    "Write",
                    "ask",
                    "denied",
                    format!("2026-05-08T12:0{idx}:00Z"),
                )
                .with_run_context(Some("session-1".into()), Some(agent.into())),
            )
            .unwrap();
        }

        let rows = filtered_denials(&db, Some("reviewer"), 1).unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].agent_type.as_deref(), Some("reviewer"));
    }

    #[test]
    fn permission_diff_reports_access_expansion() {
        let db = test_db();
        insert_profile(
            &db,
            "profile-from",
            serde_json::json!({
                "overrides": {
                    "allowed_tools": ["Read"],
                    "permission_mode": "plan",
                    "sandbox_backend": "docker"
                }
            }),
        );
        insert_profile(
            &db,
            "profile-to",
            serde_json::json!({
                "overrides": {
                    "allowed_tools": ["Read", "Bash"],
                    "skills": ["shell-helper"],
                    "permission_mode": "default",
                    "sandbox_backend": "logical"
                }
            }),
        );

        let body = permissions_cli_diff::render_permission_profile_diff_from_db(
            &db,
            "reviewer",
            "profile-from",
            "profile-to",
            false,
        )
        .unwrap();

        assert!(body.contains("Risk:  High"));
        assert!(body.contains("Expands permissions: yes"));
        assert!(body.contains("Added tools: Bash"));
        assert!(body.contains("Permission mode: plan -> default"));
        assert!(body.contains("Guardrail"));
    }

    fn insert_profile(db: &DbInstance, version_id: &str, profile_json: serde_json::Value) {
        archon_learning::agent_profile_versions::insert_agent_profile_version(
            db,
            &archon_learning::agent_profile_versions::AgentProfileVersionRecord::new(
                version_id,
                "reviewer",
                1,
                "governed_proposal",
                "2026-05-08T12:00:00Z",
            )
            .with_profile_json(profile_json),
        )
        .unwrap();
    }
}
