//! Batch O: a verifier closes a finding only with evidence, and a check
//! finding only with a failing mutation.

use super::*;

fn record(contract: Value, data: Value) -> WorkflowV2CallRecord {
    serde_json::from_value(json!({
        "call": {"id": "review-verify-t-1-9", "method": "parallel",
            "options": {"extra": {"remediationContract": contract}}},
        "attempt": 1,
        "input_hash": "h",
        "status": "needs_review",
        "result": {"status": "needs_review", "summary": "", "data": data},
    }))
    .unwrap()
}

#[test]
fn only_evidenced_dispositions_close_and_a_check_needs_a_failing_mutation() {
    let contract = json!({"version": 1, "stage": "verify", "taskId": "T", "round": 1,
        "findingIds": ["F-a", "F-b", "F-c", "F-d", "F-e"], "checkFindingIds": ["F-d", "F-e"]});
    // Dispositions inside a branch view, where a verification wave puts them.
    // The mutation F-d names ran and failed on the record; F-e's did not.
    let data = json!({"outcomes": [{"result": {"commands_run": [
        {"command": "cp -r . /tmp/m && cargo test y", "status": "failed", "exit_code": 101},
        {"command": "cargo test z", "status": "succeeded", "exit_code": 0},
    ], "data": {"finding_dispositions": [
        {"finding_id": "F-a", "disposition": "resolved", "evidence": "ran cargo test x: 3 passed"},
        {"finding_id": "F-b", "disposition": "resolved"},
        {"finding_id": "F-c", "disposition": "open", "evidence": "still reproduces"},
        {"finding_id": "F-d", "disposition": "resolved", "evidence": "test added",
         "mutation": {"command": "cargo test y", "failed": true}},
        {"finding_id": "F-e", "disposition": "resolved", "evidence": "test added",
         "mutation": {"command": "cargo test z", "failed": true}},
    ]}}}]});
    let read = reading(&record(contract, data)).unwrap();
    assert!(read["F-a"].is_ok());
    assert!(read["F-b"].as_ref().unwrap_err().contains("no evidence"));
    assert!(read["F-c"].as_ref().unwrap_err().contains("`open`"));
    assert!(read["F-d"].is_ok());
    assert!(read["F-e"].as_ref().unwrap_err().contains("mutation"));
}

#[test]
fn a_no_patch_verifier_closes_on_evidence_and_silence_is_open() {
    let contract = json!({"version": 1, "stage": "verify", "taskId": "T", "round": 1,
        "findingIds": ["F-a", "F-b", "F-c"], "refutation": true});
    let data = json!({"finding_dispositions": [
        {"finding_id": "F-a", "disposition": "invalid", "evidence": "the file never had that path"},
        {"finding_id": "F-b", "disposition": "resolved", "evidence": "looks fine"},
    ]});
    let viewed = with_remediation_dispositions(
        &record(contract, data.clone()),
        &WorkflowV2Result {
            data,
            ..WorkflowV2Result::default()
        },
    )
    .unwrap();
    let view = &viewed.data[REMEDIATION_DISPOSITIONS_KEY];
    assert_eq!(view["closed"], json!(["F-a", "F-b"]));
    assert!(
        view["open"]["F-c"]
            .as_str()
            .unwrap()
            .contains("no disposition")
    );
}

#[test]
fn a_reading_the_script_forged_is_dropped() {
    let contract = json!({"version": 1, "stage": "remediate", "taskId": "T", "round": 1});
    let data = json!({"remediation_dispositions": {"source": "host", "closed": ["F-x"]}});
    let viewed = with_remediation_dispositions(
        &record(contract, data.clone()),
        &WorkflowV2Result {
            data,
            ..WorkflowV2Result::default()
        },
    )
    .unwrap();
    assert!(viewed.data.get(REMEDIATION_DISPOSITIONS_KEY).is_none());
}

#[test]
fn check_findings_are_read_from_the_finding_itself() {
    assert!(is_check_finding(
        &json!({"claim": "The provider artifact test only checks that validation.json exists"})
    ));
    assert!(is_check_finding(&json!({"title": "no test pins KB PASS"})));
    assert!(!is_check_finding(
        &json!({"claim": "metadata says equity for a futures dataset"})
    ));
}
