use super::*;
use crate::command::workflow_decompose_remediation as remediation;

/// The entry identity every digest in `judged_check_digests` uses.
pub(super) const IDENTITY: &str = remediation::ENTRY_IDENTITY;

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
            remediation::contract_entry(&contract, &check.check_id).map(|entry| {
                (
                    check.check_id.clone(),
                    remediation::authored_entry_digest(entry),
                )
            })
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
        // The digest names the judged entry's authored bytes: the snapshot's
        // entry, never the later one, and never the whole entry.
        let entry = |bytes: &[u8]| {
            serde_json::from_slice::<archon_workflow::task_set_contract::AcceptanceContract>(bytes)
                .unwrap()
                .acceptance[0]
                .clone()
        };
        assert_eq!(
            judged["AC-X-001"],
            remediation::authored_entry_digest(&entry(snapshot))
        );
        assert_ne!(
            judged["AC-X-001"],
            remediation::authored_entry_digest(&entry(later))
        );
        assert_ne!(
            judged["AC-X-001"],
            content_digest(&serde_json::to_vec(&entry(snapshot)).unwrap())
        );
    }
}
