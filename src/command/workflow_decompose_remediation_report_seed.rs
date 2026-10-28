use anyhow::{Context, Result};
use std::path::Path;

/// Load the report, attributing older reports to their matching recording event.
pub(crate) fn read_author_report(run_dir: &Path, report_id: &str) -> Result<serde_json::Value> {
    let path = run_dir.join(format!("decomposition/reverify/reports/{report_id}.json"));
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut report: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    let event_path = run_dir.join("events.jsonl");
    if report["recorded_at"].as_str().is_none() && event_path.try_exists()? {
        let digest = archon_workflow::task_set_contract::content_digest(&bytes);
        let (events, _) = archon_workflow::events::read_event_log(&event_path)?;
        let sequence = report["report_event_seq"].as_u64().filter(|seq| *seq != 0);
        let matching = events
            .iter()
            .filter(|event| {
                event.kind == archon_workflow::WorkflowEventKind::DecompositionReverified
                    && event.detail["report_id"].as_str() == Some(report_id)
                    && event.detail["report_digest"].as_str() == Some(digest.as_str())
                    && sequence.is_none_or(|seq| event.seq == seq)
            })
            .collect::<Vec<_>>();
        // A repeated publication is not evidence of the original recording time.
        // Without a unique match, leave the attribution explicitly unknown.
        if let [event] = matching.as_slice() {
            report["recorded_at"] = serde_json::Value::String(
                event
                    .ts
                    .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
            );
        }
    }
    Ok(report)
}

pub(crate) fn report_author_seed(
    report: &serde_json::Value,
    report_id: &str,
    check_id: &str,
) -> String {
    let check = report["checks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|check| check["check_id"].as_str() == Some(check_id));
    let recorded = report["recorded_at"].as_str();
    let attribution = recorded.map_or_else(
        || format!("Report {report_id} (recording time unknown)"),
        |timestamp| format!("Report {report_id} (recorded {timestamp})"),
    );
    let mut quoted = Vec::new();
    let mut has_non_text_findings = false;
    if let Some(findings) = check.and_then(|check| check["findings"].as_array()) {
        for finding in findings {
            if let Some(finding) = finding.as_str() {
                quoted.push(serde_json::to_string(finding).expect("serializing a string"));
            } else {
                has_non_text_findings = true;
            }
        }
    }
    let statement = if check.is_none() {
        format!("Report {report_id} has no entry for check {check_id}.")
    } else if quoted.is_empty() && has_non_text_findings {
        format!(
            "Report {report_id} has findings for check {check_id} that are not text and are not quoted."
        )
    } else if quoted.is_empty() {
        format!("{attribution} has no findings for check {check_id}.")
    } else {
        format!(
            "{attribution} said about check {check_id}: {}.",
            quoted.join("; ")
        )
    };
    let verdict = match check.and_then(|check| check["verdict"].as_str()) {
        Some(verdict) => format!(
            "The report gave check {check_id} the verdict {}.",
            serde_json::to_string(verdict).expect("serializing a string")
        ),
        None => format!("The report gives no verdict for check {check_id}."),
    };
    let non_text = if has_non_text_findings && !quoted.is_empty() {
        format!(
            " Report {report_id} has findings for check {check_id} that are not text and are not quoted."
        )
    } else {
        String::new()
    };
    format!(
        "{statement}{non_text} {verdict} It describes the tree at that time and may be out of date."
    )
}
