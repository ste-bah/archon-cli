use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use archon_workflow::task_set_contract::content_digest;
use archon_workflow::v2::script::landing_lineage::LandingLineage;

/// What a resume does with every recorded landing request: per targeted
/// subject, each answer in start order, whether it is a rerun (neither
/// history nor authority), the current body (the one request a resume may
/// judge again) or a superseded round (replayed from its record, nothing
/// published).
pub(crate) fn landing_report(run_dir: &Path) -> Result<Vec<String>> {
    let records =
        archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2")).load_call_records()?;
    let subjects: BTreeSet<(String, String)> = records
        .iter()
        .filter_map(|record| record.call.options.host_command.as_ref())
        .filter_map(|request| {
            request
                .target
                .as_ref()
                .map(|target| (request.command_id.clone(), target.task_id.clone()))
        })
        .collect();
    let mut lines = Vec::new();
    for (command, subject) in &subjects {
        let lineage = LandingLineage::of(&records, command, subject);
        let current = lineage.current();
        let current_occurrence =
            current.map(archon_workflow::v2::script::landing_lineage::occurrence_of);
        lines.push(format!(
            "landing lineage {command} {subject}: current body {}",
            current.map_or("none", |record| record.call.id.as_str())
        ));
        for record in lineage.answers() {
            let Some(request) = record.call.options.host_command.as_ref() else {
                continue;
            };
            let occurrence = archon_workflow::v2::script::landing_lineage::occurrence_of(record);
            let replayed = lineage.superseded_answer(request, occurrence, |_| false);
            let role = if lineage.is_rerun(record) {
                "rerun: neither history nor authority".to_string()
            } else if current.is_some_and(|current| {
                current.call.options.host_command.as_ref() == Some(request)
                    && current_occurrence == Some(occurrence)
            }) {
                "current body's request: the one request a resume may judge again".to_string()
            } else {
                format!(
                    "superseded round: asked again it replays {} and publishes nothing",
                    replayed.map_or("none", |answer| answer.call.id.as_str())
                )
            };
            lines.push(format!(
                "  {} {} status={} published={} stdin_blake3={}: {role}",
                record.started_at,
                record.call.id,
                serde_json::to_value(record.status)?
                    .as_str()
                    .unwrap_or_default(),
                !record.result.data["publicationReceipt"].is_null(),
                content_digest(request.stdin.as_deref().unwrap_or_default().as_bytes()),
            ));
        }
    }
    Ok(lines)
}
