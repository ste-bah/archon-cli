//! The shadow records of a committed staged call, split out of
//! `workflow_gate.rs` to hold the 500-line ceiling.

use super::*;

/// Records the findings of a call the parent has already committed.
///
/// The staged child must not write here. It only ever produces a prepared
/// publication, and the parent refuses that publication on non-zero exit,
/// digest mismatch, sentinel violation or timeout. A child that appended its
/// findings first left them behind on exactly those refusals, so the log
/// carried evidence from a call that never landed.
pub(crate) fn append_published_shadow_records(
    cwd: &Path,
    call_id: &str,
    gate_label: &str,
    findings: &[archon_workflow::GatePolicyFinding],
    mode: &'static str,
) -> Result<()> {
    if findings.is_empty() {
        return Ok(());
    }
    let path = shadow_log_path(cwd);
    let parent = path.parent().expect("shadow log has a parent");
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "creating workflow gate shadow log directory {}",
            parent.display()
        )
    })?;
    let timestamp = chrono::Utc::now().to_rfc3339();
    let mut lines = Vec::with_capacity(findings.len());
    for finding in findings {
        let record = ShadowRecord {
            schema_version: "workflow-gate-shadow-v1",
            gate_id: gate_label,
            finding: &finding.text,
            subject: &finding.subject,
            source_path: finding.source_path.clone(),
            mode,
            timestamp: timestamp.clone(),
            binary_commit: env!("ARCHON_GIT_HASH"),
            call_id: Some(call_id),
        };
        lines.push(serde_json::to_string(&record)?);
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening workflow gate shadow log {}", path.display()))?;
    append_serialized_records(&mut file, &lines)
        .with_context(|| format!("writing workflow gate shadow log {}", path.display()))
}
