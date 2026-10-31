//! #549 at the whole-set freeze: a check whose own named files exceed the
//! judged-source limit goes back to its author with what it names and the
//! limit; it is never refuted on unseen content and never stops the run.

use super::*;
use archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE;

#[tokio::test]
async fn an_oversize_check_is_its_authors_finding_at_freeze_549() {
    let temp = tempfile::tempdir().unwrap();
    let tasks = temp.path().join("tasks/PRD-X");
    std::fs::create_dir_all(&tasks).unwrap();
    let prd = temp.path().join("prds/PRD-X.md");
    std::fs::create_dir_all(prd.parent().unwrap()).unwrap();
    std::fs::write(
        &prd,
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n",
    )
    .unwrap();
    std::fs::write(
        temp.path().join("out.json"),
        format!(r#"{{"valid":true,"pad":"{}"}}"#, "a".repeat(BUDGET)),
    )
    .unwrap();
    let contract = br#"{
      "schema_version":1,
      "prd":{"path":"prds/PRD-X.md","digest":"pending"},
      "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
      "acceptance":[{
        "id":"AC-X-001","criterion":"output is valid",
        "check":{"kind":"command","command":"jq -e '.valid == true' out.json","cwd":"project_root"},
        "gap_permitted":false,
        "judgment":{"verdict":"refuted","counterexample":"untrusted","reason":"untrusted","host_call_id":"untrusted"}
      }],
      "supplementary":[]
    }"#;
    std::fs::write(tasks.join(ACCEPTANCE_CONTRACT_FILE), contract).unwrap();
    crate::command::workflow_task_set::passability_tests_support::commit_project(temp.path());
    let client = std::sync::Arc::new(Recording::default());
    let prepared = crate::command::workflow_task_set::prepare_acceptance_freeze(
        temp.path(),
        &tasks,
        &prd,
        archon_core::config::GateMode::Observe,
        client.clone(),
    )
    .await
    .unwrap();
    let texts = (prepared.findings.iter())
        .map(|finding| finding.text.as_str())
        .collect::<Vec<_>>();
    assert!(
        !texts.iter().any(|text| text.contains("was refuted")),
        "{texts:?}"
    );
    assert!(
        (prepared.findings.iter()).all(|finding| {
            finding.remediation_scope != archon_workflow::RemediationScope::Operational
        }),
        "no operational finding stops the phase: {texts:?}"
    );
    let size = std::fs::metadata(temp.path().join("out.json"))
        .unwrap()
        .len();
    let author = (prepared.findings.iter())
        .find(|finding| finding.subject == "AC-X-001")
        .unwrap_or_else(|| panic!("the author receives a finding: {texts:?}"));
    assert_eq!(
        author.remediation_scope,
        archon_workflow::RemediationScope::CandidateArtifact
    );
    assert_eq!(
        author.text,
        format!(
            "check 'AC-X-001' names {size} bytes of source (out.json {size} bytes), more than the judge can read ({BUDGET} bytes); name only the files whose content decides the criterion, or decide it with a command that does not need those files judged"
        )
    );
    assert!(
        (client.prompts.lock().unwrap().iter()).all(|prompt| !prompt.contains("AC-X-001")),
        "the judge is never asked about the check"
    );
    assert!(prepared.non_accepted.contains("AC-X-001"));
}
