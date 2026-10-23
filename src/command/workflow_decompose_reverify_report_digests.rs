use super::*;

pub(super) fn for_report(
    judged_contract_bytes: &[u8],
    v2: &WorkflowV2ResultStore,
    report_id: &str,
    checks: &[acceptance::CheckResult],
) -> Result<(
    Vec<String>,
    String,
    std::collections::BTreeMap<String, String>,
)> {
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(judged_contract_bytes)
            .context("parsing the judged acceptance contract snapshot")?;
    let judged = checks
        .iter()
        .filter_map(|check| {
            contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .find(|entry| entry.id == check.check_id)
                .and_then(|entry| {
                    serde_json::to_vec(&serde_json::json!({
                        "id": &entry.id, "criterion": &entry.criterion, "check": &entry.check,
                        "covers": &entry.covers, "gap_permitted": entry.gap_permitted,
                    }))
                    .ok()
                })
                .map(|entry| (check.check_id.clone(), content_digest(&entry)))
        })
        .collect();
    let mut calls = v2
        .load_call_records()?
        .into_iter()
        .map(|record| record.call.id)
        .collect::<Vec<_>>();
    calls.push(report_id.to_string());
    calls.sort();
    Ok((calls, content_digest(judged_contract_bytes), judged))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_digests_keep_the_contract_snapshot_captured_before_report_writing() {
        let store_root = tempfile::tempdir().unwrap();
        let snapshot = br#"{"schema_version":1,"prd":{"path":"PRD.md","digest":"p"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":[{"id":"AC-X-001","criterion":"before","check":{"kind":"command","command":"true","cwd":"project_root"},"covers":[],"gap_permitted":false,"judgment":{"verdict":"accepted","counterexample":"x","reason":"y","host_call_id":"j"}}],"supplementary":[]}"#;
        let later = br#"{"schema_version":1,"prd":{"path":"PRD.md","digest":"p"},"gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},"acceptance":[{"id":"AC-X-001","criterion":"after","check":{"kind":"command","command":"true","cwd":"project_root"},"covers":[],"gap_permitted":false,"judgment":{"verdict":"accepted","counterexample":"x","reason":"y","host_call_id":"j"}}],"supplementary":[]}"#;
        let checks = [acceptance::CheckResult {
            check_id: "AC-X-001".into(),
            verdict: "needs_remediation".into(),
            really_ran: true,
            setup_failure: false,
            real_failure: true,
            sibling_paths: Vec::new(),
            findings: Vec::new(),
            not_run_reason: None,
            regression_guard: None,
            remediation_route: None,
        }];
        let (_, contract_digest, judged) = for_report(
            snapshot,
            &WorkflowV2ResultStore::new(store_root.path()),
            "report",
            &checks,
        )
        .unwrap();
        assert_eq!(contract_digest, content_digest(snapshot));
        let before: serde_json::Value = serde_json::from_slice(snapshot).unwrap();
        let after: serde_json::Value = serde_json::from_slice(later).unwrap();
        assert_ne!(
            before["acceptance"][0]["criterion"],
            after["acceptance"][0]["criterion"]
        );
        assert_eq!(judged.len(), 1);
        assert!(judged["AC-X-001"] != content_digest(later));
    }
}
