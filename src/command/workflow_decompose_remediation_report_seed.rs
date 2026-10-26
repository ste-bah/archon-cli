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
        || format!("Report {report_id}"),
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
