//! Pin lineage proof for consumed remediation reports.

use super::*;

/// Re-author a consumed subject when the recorded report was consumed and its
/// authored entry digest is still present in the live contract. This proves
/// that the report's flagged authored bytes remain; it does not prove an
/// intervening repair followed by a return. Missing lineage is not evidence
/// that a repair was lost.
pub(super) fn consumed_ledger_findings(
    task_root: &Path,
    run_dir: &Path,
    ledger: &Ledger,
) -> Result<std::collections::BTreeMap<String, String>> {
    consumed_ledger_review(task_root, run_dir, ledger).map(|(findings, _)| findings)
}

/// [`consumed_ledger_findings`] with one operator line per consumed subject
/// it does not route, saying why.
pub(super) fn consumed_ledger_review(
    task_root: &Path,
    run_dir: &Path,
    ledger: &Ledger,
) -> Result<(std::collections::BTreeMap<String, String>, Vec<String>)> {
    let contract_path =
        task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let contract_bytes = std::fs::read(&contract_path)
        .with_context(|| format!("reading {}", contract_path.display()))?;
    let current: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&contract_bytes)
            .with_context(|| format!("parsing {}", contract_path.display()))?;
    let mut findings = std::collections::BTreeMap::new();
    let mut skipped = Vec::new();
    let consumed_path = run_dir.join("decomposition/reverify/consumed.jsonl");
    let consumed_text = match std::fs::read_to_string(&consumed_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", consumed_path.display()));
        }
    };
    // The same tolerant reader `is_consumed` and `prepare` use: an
    // interrupted final append is not a consumption, and never an error.
    let is_consumed_here = resolved::resolved_rows(&consumed_text, &consumed_path)?
        .iter()
        .any(|row| row["report_id"].as_str() == Some(ledger.report_id.as_str()));
    if !is_consumed_here {
        return Ok((findings, skipped));
    }
    let report_path = run_dir.join(format!(
        "decomposition/reverify/reports/{}.json",
        ledger.report_id
    ));
    let report_bytes = match std::fs::read(&report_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(report = %ledger.report_id, "consumed reverify report has no recorded judged digests; heal needs the next reverify");
            return Ok((findings, skipped));
        }
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", report_path.display()));
        }
    };
    let report: serde_json::Value = serde_json::from_slice(&report_bytes)
        .with_context(|| format!("parsing {}", report_path.display()))?;
    let judged_digests = report["judged_check_digests"].as_object();
    if report["contract_digest"].as_str().is_none() || judged_digests.is_none() {
        tracing::info!(report = %ledger.report_id, "consumed reverify report has no recorded judged digests; heal needs the next reverify");
        return Ok((findings, skipped));
    }
    let judged_digests = judged_digests.expect("checked above");
    // A report a previous build wrote has no identity version: its digests
    // cover the whole entry, and they still name the entry they judged.
    let versioned = report["judged_check_identity"].as_str() == Some(resolved::ENTRY_IDENTITY);
    for id in &ledger.subjects {
        let current_entry = resolved::contract_entry(&current, id);
        let verdict = report["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|check| check["check_id"].as_str() == Some(id.as_str()))
            .and_then(|check| check["verdict"].as_str());
        let judged_digest = judged_digests.get(id).and_then(serde_json::Value::as_str);
        let flagged_bytes_back = judged_digest
            .zip(current_entry)
            .is_some_and(|(judged, entry)| {
                resolved::recorded_digest_names(judged, versioned, entry)
            });
        if verdict == Some("needs_remediation") && flagged_bytes_back {
            findings.insert(id.clone(), format!("consumed reverify report {} flagged this exact authored check entry; the flagged bytes are back, so re-authoring it through judge and host proof", ledger.report_id));
        } else {
            skipped.push(format!(
                "consumed reverify report {} subject {id} is not re-authored: {}",
                ledger.report_id,
                if current_entry.is_none() {
                    "it is not in the published contract".to_string()
                } else if verdict != Some("needs_remediation") {
                    format!("the report's verdict was {}", verdict.unwrap_or("absent"))
                } else if judged_digest.is_none() {
                    "the report recorded no judged digest for it".to_string()
                } else {
                    "the published entry is not the one the report flagged".to_string()
                }
            ));
        }
    }
    Ok((findings, skipped))
}
