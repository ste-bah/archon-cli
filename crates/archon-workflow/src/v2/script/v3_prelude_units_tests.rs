//! Issue 219 item 1: review findings reach remediation whole.
//!
//! The run that ended Accepted with 17 open defects cut each task's finding
//! list at 6000 characters with no marker, so finding #17 of 19 never
//! reached a writer or a verifier. The prelude now splits a unit's findings
//! into several units of WHOLE findings (`chunkFindings`) and puts each
//! unit's list in its prompt unsliced. This runs that splitter on the shape
//! that lost findings, without a recorded run.

fn chunk_js(driver: &str) -> String {
    let prelude = super::V3_PRIMITIVES_JS;
    let start = prelude
        .find("  const REMEDIATION_UNIT_CHARS = ")
        .expect("the unit budget must exist");
    let end = start
        + prelude[start..]
            .find("  const planUnits = ")
            .expect("planUnits must follow chunkFindings");
    let script = format!("{}\n{driver}\n", &prelude[start..end]);
    let dir = tempfile::tempdir().expect("tmp");
    let path = dir.path().join("units.mjs");
    std::fs::write(&path, script).expect("write driver");
    let out = std::process::Command::new("node")
        .arg(&path)
        .output()
        .expect("node must be available");
    assert!(
        out.status.success(),
        "driver failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 19 findings of ~700 characters (past the old 6000-character cut) and
/// one larger than a whole unit: every finding comes back once, in
/// order, byte-for-byte, and the large one is a unit of its own.
#[test]
fn nineteen_findings_and_an_oversized_one_all_reach_units_whole() {
    let driver = r#"
const own = [];
for (let i = 1; i <= 19; i += 1) own.push({ finding_id: `F${i}`, claim: `defect ${i} `.repeat(60) });
own.splice(9, 0, { finding_id: "BIG", claim: "x".repeat(20000) });
const chunks = chunkFindings(own);
const flat = chunks.flat();
console.log(JSON.stringify({
  every_finding_once_in_order: JSON.stringify(flat) === JSON.stringify(own),
  big_alone: chunks.some((c) => c.length === 1 && c[0].finding_id === "BIG"),
  more_than_one_unit: chunks.length > 1,
  no_unit_over_budget_but_a_single_finding: chunks.every((c) => c.length === 1 || JSON.stringify(c).length <= REMEDIATION_UNIT_CHARS + 2 * c.length),
}));"#;
    assert_eq!(
        chunk_js(driver),
        r#"{"every_finding_once_in_order":true,"big_alone":true,"more_than_one_unit":true,"no_unit_over_budget_but_a_single_finding":true}"#
    );
}

/// The fix prompt carries a unit's open findings unsliced: no
/// `.slice(` is applied to the findings text anywhere in the prelude's
/// remediation round.
#[test]
fn the_fix_prompt_carries_the_unit_findings_unsliced() {
    let prelude = super::V3_PRIMITIVES_JS;
    let line = prelude
        .lines()
        .find(|line| line.contains("const verbatim = "))
        .expect("the verbatim findings line must exist");
    assert!(line.contains("JSON.stringify(open)"), "{line}");
    assert!(!line.contains(".slice("), "{line}");
    assert!(!prelude.contains(".slice(0, 6000)"));
}

/// Issue 219 round 3: a demoted branch's fix prompt (the completion
/// unit's verbatim rejected envelope) carries the path of the evidence
/// file that holds every declared-contract finding.
#[test]
fn the_fix_prompt_names_the_file_holding_every_contract_finding() {
    let run = tempfile::tempdir().expect("tmp");
    let findings: Vec<String> = (1..=1000)
        .map(|n| format!("records[{n}] declared instance is missing from the artifact"))
        .collect();
    let mut outcome = crate::v2::WorkflowV2BranchOutcome {
        item_id: "TASK-1".to_string(),
        role: "implementer".to_string(),
        status: crate::v2::WorkflowV2Status::Accepted,
        result: Some(crate::v2::WorkflowV2Result::accepted("done")),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    };
    crate::v2::verification::demote_failed_contract(&mut outcome, &findings, Some(run.path()));
    let result = outcome.result.expect("result");
    let path = result.data["declared_contract_findings_path"]
        .as_str()
        .expect("path")
        .to_string();
    let envelope = serde_json::json!({ "result": result });
    let prelude = super::V3_PRIMITIVES_JS;
    let start = prelude
        .find("  const completionText = ")
        .expect("completionText");
    let end = prelude
        .find("  const completionSaid = ")
        .expect("completionSaid");
    let driver = format!(
        "{}\nconst rejected = `Implementation envelope:\\n${{completionText({envelope})}}`;\nprocess.stdout.write(`Remediate TASK-1. The previous attempt was REJECTED. Fix exactly what these verbatim envelopes name:\\n${{rejected}}`);\n",
        &prelude[start..end]
    );
    let dir = tempfile::tempdir().expect("tmp");
    let script = dir.path().join("prompt.mjs");
    std::fs::write(&script, driver).expect("write driver");
    let out = std::process::Command::new("node")
        .arg(&script)
        .output()
        .expect("node must be available");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let prompt = String::from_utf8_lossy(&out.stdout);
    // The prompt quotes the envelope as JSON, so a Windows path's
    // backslashes appear escaped.
    let quoted = serde_json::to_string(&path).expect("path json");
    let quoted = &quoted[1..quoted.len() - 1];
    assert!(
        prompt.contains(&format!("full list at {quoted}")),
        "{prompt}"
    );
    assert!(prompt.len() < 40 * 1024, "{}", prompt.len());
}
