//! A check the judge refuted is never published: it goes back to its author.

use super::*;
use crate::command::workflow_task_set::reauthor::AuthorScope;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};

const ORIGINAL: &str = "jq -e '.valid == true' out.json";
const STRONGER: &str = "jq -e '.valid == true and (.rows | length) > 0' out.json";

fn publish(project: &Path, prepared: PreparedAcceptanceFreeze) -> Result<FreezeAcceptanceResult> {
    let findings = prepared.findings.clone();
    let identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        project,
        GateMode::Observe,
        GateId::FreezeAcceptance,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(identity),
            )
        },
    )?;
    let permit = disposition.take_publication_permit().unwrap();
    publish_acceptance_freeze(prepared, permit)
}

#[tokio::test]
async fn whole_set_freeze_returns_a_refuted_check_to_its_author_before_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let client = Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, STRONGER),
        |_, check| check["command"] != serde_json::json!(ORIGINAL),
    ));
    let scope = AuthorScope::for_task_set(temp.path(), &tasks, &prd);
    let prepared = prepare_acceptance_freeze_reauthoring(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        client.clone(),
        &scope,
    )
    .await
    .expect("the re-authored check is accepted");
    assert!(prepared.non_accepted_ids().is_empty());
    assert_eq!(client.authored(), 1);
    publish(temp.path(), prepared).expect("an all-accepted freeze publishes");
    let contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
            .unwrap();
    let entry = &contract.acceptance[0];
    assert_eq!(
        entry.judgment.verdict,
        archon_workflow::task_set_contract::JudgeDecision::Accepted
    );
    assert!(matches!(
        &entry.check,
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } if command == STRONGER
    ));
}

#[tokio::test]
async fn whole_set_freeze_fails_bounded_with_nothing_written_when_the_judge_keeps_refuting() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, original) = seed(&temp);
    let client = Arc::new(ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, &format!("jq -e '.v{attempt} == true' out.json")),
        |_, _| false,
    ));
    let scope = AuthorScope::for_task_set(temp.path(), &tasks, &prd);
    let error = prepare_acceptance_freeze_reauthoring(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        client.clone(),
        &scope,
    )
    .await
    .expect_err("still refuted after the bound")
    .to_string();
    assert!(error.contains("AC-X-001"), "{error}");
    assert!(error.contains("after 3 re-author attempt(s)"), "{error}");
    assert_eq!(client.authored(), 3);
    assert_eq!(
        std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        original
    );
    assert!(!tasks.join(ACCEPTANCE_LOCK_FILE).exists());
    assert!(!acceptance_pin_path(temp.path(), &tasks).exists());
}

#[tokio::test]
async fn a_refuted_check_is_never_staged_and_its_finding_returns_to_the_author() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, _, candidate) = seed(&temp);
    // The criterion prescribes the check's shape, which used to make its
    // findings observations the author was never sent.
    let prd = temp.path().join("prds/PRD-X.md");
    std::fs::write(
        &prd,
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | the typed_verifier_command passes |\n",
    )
    .unwrap();
    let prepared = prepare_acceptance_freeze_from_candidate(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        candidate,
        Arc::new(JudgeClient {
            result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"refuted","counterexample":"a passing false state","reason":"weak"}]}"#.into()),
        }),
    )
    .await
    .unwrap();
    let (evaluation, outputs) = prepared.into_staged_parts();
    assert!(
        outputs.is_empty(),
        "a refuted contract stages the envelope alone"
    );
    let finding = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.contains("was refuted by the host judge"))
        .expect("the judge's refutation is a finding");
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::CandidateArtifact
    );
    assert!(
        finding.text.contains("a passing false state"),
        "{}",
        finding.text
    );
}
