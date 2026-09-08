//! Consume only host-persisted findings; a disposition is not resolution.
use super::*;
use crate::repository_audit::{RequiredAction};
use serde::Deserialize;

use crate::repository_audit::reuse::load_state as state;
pub(super) fn preamble(store:&WorkflowV2ResultStore, paths:&[String])->WorkflowResult<String> {
    let Some(state)=state(store)? else {return Ok(String::new());};
    let Some(report)=state.ledger.history.last() else {return Ok(String::new());};
    let mut lines=Vec::new();
    for record in report.records.iter().filter(|r|paths.contains(&r.declared_path)) {
        lines.push(serde_json::to_string(&serde_json::json!({"snapshot":report.snapshot,"declared_path":record.declared_path,
            "verdict":record.verdict,"equivalents":record.equivalents,"required_action":record.required_action,"operator_waived":state.ledger.is_waived(&record.declared_path,&report.snapshot)}))?);
    }
    if lines.is_empty(){return Ok(String::new());}
    Ok(format!("\nHost repository audit (equivalents are read context, NOT write permission):\n{}\nFor actionable findings return data.audit_dispositions entries with declared_path, snapshot, explanation and evidence_paths. These are proposals, not resolution; actual applied changes are assessed independently.\n",lines.join("\n")))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Disposition {
    pub(super) declared_path: String,
    pub(super) snapshot: String,
    pub(super) explanation: String,
    pub(super) evidence_paths: Vec<String>,
}

pub(super) fn applied_dispositions(
    result: &WorkflowV2Result, manifest: &PatchManifest, snapshot: &str,
) -> Vec<Disposition> {
    let dispositions: Vec<Disposition> = serde_json::from_value(
        result.data.get("audit_dispositions").cloned().unwrap_or(serde_json::json!([])))
        .unwrap_or_default();
    dispositions.into_iter().filter(|d| d.snapshot == snapshot
        && manifest.declared_target_files.contains(&d.declared_path)
        && !d.explanation.trim().is_empty() && d.explanation.len() <= 2048
        && !d.evidence_paths.is_empty()
        && d.evidence_paths.iter().all(|p| manifest.changed_files.iter()
            .chain(&manifest.created_files).chain(&manifest.deleted_files).any(|changed| changed == p)))
        .collect()
}

pub(super) fn enforce(
    store:&WorkflowV2ResultStore, owned:&[String], result:&mut WorkflowV2Result,
    manifest:&mut Option<PatchManifest>,
)->WorkflowResult<()> {
    if !matches!(result.status,WorkflowV2Status::Accepted|WorkflowV2Status::Noop){return Ok(());}
    let Some(state)=state(store)? else {return Ok(());};
    let Some(report)=state.ledger.history.last() else {return Err(WorkflowError::StateCorrupt("audit ledger has no assessment".into()));};
    let dispositions:Vec<Disposition>=serde_json::from_value(result.data.get("audit_dispositions").cloned().unwrap_or(serde_json::json!([])))
        .unwrap_or_default();
    let mut gaps=Vec::new();
    for record in report.records.iter().filter(|r|owned.contains(&r.declared_path)&&r.required_action==RequiredAction::WireOrMigrate) {
        if state.ledger.is_waived(&record.declared_path, &report.snapshot) { continue; }
        let matches=dispositions.iter().filter(|d|d.declared_path==record.declared_path).collect::<Vec<_>>();
        let explained=matches.len()==1 && matches[0].snapshot==report.snapshot && !matches[0].explanation.trim().is_empty()
            && matches[0].explanation.len()<=2048 && !matches[0].evidence_paths.is_empty()
            && matches[0].evidence_paths.iter().all(|p| manifest.as_ref().is_some_and(|m|
                m.changed_files.contains(p)||m.created_files.contains(p)||m.deleted_files.contains(p)));
        if !explained {gaps.push(record.declared_path.clone());}
    }
    // Naming an equivalent in the audit must not activate automatic scope grants.
    if let Some(m)=manifest.as_ref() {
        for equivalent in report.records.iter().filter(|r|owned.contains(&r.declared_path)).flat_map(|r|&r.equivalents) {
            if !owned.contains(equivalent) && (m.changed_files.contains(equivalent)||m.created_files.contains(equivalent)||m.deleted_files.contains(equivalent)) {
                gaps.push(format!("{equivalent} (equivalent is outside declared ownership)"));
            }
        }
    }
    if gaps.is_empty(){return Ok(());}
    let reason=format!("repository audit rejected unexplained or unauthorized changes: {}",gaps.join(", "));
    if let Some(m)=manifest.as_mut() {
        m.status=ManifestStatus::Failed{reason:reason.clone()};
        let path=PathBuf::from(manifest_path_for(store.root().parent().unwrap(),&m.stage_id,&m.item_id));
        std::fs::write(&path,serde_json::to_vec_pretty(m)?).map_err(|e|WorkflowError::io(path,e))?;
    }
    *manifest=None;
    result.status=WorkflowV2Status::NeedsReview;
    result.summary=reason.clone();
    result.residual_gaps.push(WorkflowV2ResidualGap{id:"repository_audit_unaddressed".into(),description:reason,severity:Some("blocking".into())});
    Ok(())
}
