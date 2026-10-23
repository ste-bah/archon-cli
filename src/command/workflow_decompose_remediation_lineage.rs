//! Pin lineage proof for consumed remediation reports.

use super::*;

fn authored_entry_digest(
    entry: &archon_workflow::task_set_contract::AcceptanceCriterion,
) -> anyhow::Result<String> {
    Ok(archon_workflow::task_set_contract::content_digest(
        &serde_json::to_vec(&serde_json::json!({
            "id": &entry.id,
            "criterion": &entry.criterion,
            "check": &entry.check,
            "covers": &entry.covers,
            "gap_permitted": entry.gap_permitted,
        }))?,
    ))
}

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
    let contract_path =
        task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let contract_bytes = std::fs::read(&contract_path)
        .with_context(|| format!("reading {}", contract_path.display()))?;
    let current: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&contract_bytes)
            .with_context(|| format!("parsing {}", contract_path.display()))?;
    let mut findings = std::collections::BTreeMap::new();
    let consumed_path = run_dir.join("decomposition/reverify/consumed.jsonl");
    let consumed_text = match std::fs::read_to_string(&consumed_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("reading {}", consumed_path.display()));
        }
    };
    let is_consumed_here = consumed_text
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let row: serde_json::Value = serde_json::from_str(line).with_context(|| {
                format!("parsing line {} of {}", index + 1, consumed_path.display())
            })?;
            Ok(row["report_id"].as_str() == Some(ledger.report_id.as_str()))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .any(|matches| matches);
    if !is_consumed_here {
        return Ok(findings);
    }
    let report_path = run_dir.join(format!(
        "decomposition/reverify/reports/{}.json",
        ledger.report_id
    ));
    let report_bytes = match std::fs::read(&report_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::info!(report = %ledger.report_id, "consumed reverify report has no recorded judged digests; heal needs the next reverify");
            return Ok(findings);
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
        return Ok(findings);
    }
    let judged_digests = judged_digests.expect("checked above");
    for id in &ledger.subjects {
        let current_entry = current
            .acceptance
            .iter()
            .chain(&current.supplementary)
            .find(|entry| &entry.id == id);
        let verdict = report["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|check| check["check_id"].as_str() == Some(id.as_str()))
            .and_then(|check| check["verdict"].as_str());
        let judged_digest = judged_digests.get(id).and_then(serde_json::Value::as_str);
        let current_digest = current_entry.map(authored_entry_digest).transpose()?;
        if verdict == Some("needs_remediation")
            && judged_digest.is_some_and(|judged| current_digest.as_deref() == Some(judged))
        {
            findings.insert(id.clone(), format!("consumed reverify report {} flagged this exact authored check entry; the flagged bytes are back, so re-authoring it through judge and host proof", ledger.report_id));
        } else {
            tracing::info!(subject = %id, ledger = %ledger.report_id, "skipping consumed subject: current authored entry does not match a flagged judged digest");
        }
    }
    Ok(findings)
}
