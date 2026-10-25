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
    let scope = AuthorScope::for_task_set(temp.path(), &tasks, &prd)
        .expect("the task set's repository record is believed");
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
    let scope = AuthorScope::for_task_set(temp.path(), &tasks, &prd)
        .expect("the task set's repository record is believed");
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

#[cfg(unix)]
#[tokio::test]
async fn a_judge_refutation_reaches_its_author_while_a_sibling_is_host_unproven() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, candidate) = seed(&temp);
    std::fs::write(
        &prd,
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n| AC-X-002 | output is complete |\n",
    )
    .unwrap();
    let mut candidate: serde_json::Value = serde_json::from_slice(&candidate).unwrap();
    candidate["acceptance"][0]["check"]["command"] = serde_json::json!("test -f output.json");
    let mut sibling = candidate["acceptance"][0].clone();
    sibling["id"] = serde_json::json!("AC-X-002");
    sibling["criterion"] = serde_json::json!("output is complete");
    sibling["check"]["command"] = serde_json::json!("sleep 30; test -f output.json");
    candidate["acceptance"]
        .as_array_mut()
        .unwrap()
        .push(sibling);

    let scratch = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join(".archon")).unwrap();
    std::fs::write(
        temp.path().join(".archon/config.toml"),
        format!(
            "[workflow.acceptance_execution]\nrepository={:?}\nscratch_parent={:?}\nproject_inputs=[]\nproject_repository_view=\"separate\"\ntoolchain_path=\"/usr/bin:/bin:/usr/sbin:/sbin\"\ntimeout_secs=3\noutput_bytes=8192\nscratch_bytes=16777216\n",
            temp.path(), scratch.path()
        ),
    )
    .unwrap();

    let client = Arc::new(JudgeClient {
        result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"refuted","counterexample":"a passing false state","reason":"the check misses a case"},{"id":"AC-X-002","verdict":"accepted","counterexample":"none constructible","reason":"the check observes the criterion"}]}"#.into()),
    });
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let prepared = prepare_acceptance_freeze_resumable(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&candidate).unwrap(),
        client,
        &resume,
    )
    .await
    .unwrap_or_else(|error| {
        panic!("A's finding must be returned while B stays pending: {error:#}")
    });

    let refutation = prepared
        .findings
        .iter()
        .find(|finding| finding.subject == "AC-X-001")
        .expect("A's author receives the judge refutation in this round");
    assert!(
        refutation.text.contains("was refuted by the host judge"),
        "{}",
        refutation.text
    );
    assert_eq!(
        refutation.remediation_scope,
        archon_workflow::RemediationScope::CandidateArtifact
    );
    let pending = prepared
        .findings
        .iter()
        .find(|finding| finding.subject == "AC-X-002")
        .expect("B remains host-unproven");
    assert_eq!(
        pending.remediation_scope,
        archon_workflow::RemediationScope::Operational
    );
    assert!(pending.text.contains("could not prove"), "{}", pending.text);
    let judge_cache = temp
        .path()
        .join(crate::command::workflow_freeze_budget::FREEZE_CACHE_DIR)
        .join("judge");
    let saved: Vec<serde_json::Value> = std::fs::read_dir(judge_cache)
        .unwrap()
        .map(|entry| {
            serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap()
        })
        .collect();
    assert!(
        saved.iter().any(|record| {
            let entries = record["judged"]["acceptance"].as_array();
            entries.is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry["id"] == "AC-X-001" && entry["judgment"]["verdict"] == "refuted"
                }) && entries.iter().any(|entry| {
                    entry["id"] == "AC-X-002" && entry["judgment"]["verdict"] == "accepted"
                })
            })
        }),
        "the judged verdicts remain in the staged freeze cache"
    );
    let (evaluation, outputs) = prepared.into_staged_parts();
    assert!(outputs.is_empty(), "an incomplete set is not published");
    assert_eq!(evaluation.findings.len(), 2);
}

#[cfg(unix)]
fn staged_host_unproven_fixture(tasks: &Path, prd: &Path, cover_all: bool) -> serde_json::Value {
    std::fs::write(
        prd,
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n| AC-X-002 | output is complete |\n\n- REQ-X-001: output is valid.\n- REQ-X-002: output is complete.\n",
    )
    .unwrap();
    let mut candidate: serde_json::Value =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
            .unwrap();
    candidate["acceptance"][0]["check"]["command"] = serde_json::json!("test -f output.json");
    candidate["acceptance"][0]["covers"] = if cover_all {
        serde_json::json!(["REQ-X-001", "REQ-X-002"])
    } else {
        serde_json::json!(["REQ-X-001"])
    };
    let mut sibling = candidate["acceptance"][0].clone();
    sibling["id"] = serde_json::json!("AC-X-002");
    sibling["criterion"] = serde_json::json!("output is complete");
    // A first-strike signal is a real host-unproven result, independent of
    // whether a timeout is classified as a host fault.
    sibling["check"]["command"] = serde_json::json!("kill -TERM $$");
    candidate["acceptance"]
        .as_array_mut()
        .unwrap()
        .push(sibling);
    candidate
}

#[cfg(unix)]
#[tokio::test]
async fn placeholder_refutations_do_not_prevent_a_stall_when_the_judge_accepts_everything() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let candidate = staged_host_unproven_fixture(&tasks, &prd, true);
    // These untrusted draft placeholders claim refutation. The actual judge
    // accepts both checks, so no authored finding may be manufactured from
    // the placeholder values.
    let client = Arc::new(JudgeClient {
        result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"accepted"},{"id":"AC-X-002","verdict":"accepted","counterexample":"none","reason":"accepted"}]}"#.into()),
    });
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let error = prepare_acceptance_freeze_resumable(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&candidate).unwrap(),
        client,
        &resume,
    )
    .await
    .expect_err("only an operational finding remains, so the freeze stalls");
    assert!(
        crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error).is_some(),
        "{error:#}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn uncovered_requirement_reaches_author_while_sibling_is_host_unproven() {
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let candidate = staged_host_unproven_fixture(&tasks, &prd, false);
    let client = Arc::new(JudgeClient {
        result: Ok(r#"{"decisions":[{"id":"AC-X-001","verdict":"accepted","counterexample":"none","reason":"accepted"},{"id":"AC-X-002","verdict":"accepted","counterexample":"none","reason":"accepted"}]}"#.into()),
    });
    let resume = crate::command::workflow_freeze_budget::FreezeResume::staged("freeze-acceptance");
    let prepared = prepare_acceptance_freeze_resumable(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        serde_json::to_vec(&candidate).unwrap(),
        client,
        &resume,
    )
    .await
    .unwrap_or_else(|error| panic!("coverage findings must reach the author: {error:#}"));
    assert!(
        prepared.findings.iter().any(|finding| {
            finding.text.contains("covered by no acceptance check")
                && finding.remediation_scope == archon_workflow::RemediationScope::CandidateArtifact
        }),
        "{:?}",
        prepared.findings
    );
    assert!(
        prepared.findings.iter().any(|finding| {
            finding.subject == "AC-X-002"
                && finding.remediation_scope == archon_workflow::RemediationScope::Operational
        }),
        "{:?}",
        prepared.findings
    );
    let (evaluation, outputs) = prepared.into_staged_parts();
    assert_eq!(
        outputs.len(),
        3,
        "the staged candidate is not published yet"
    );
    assert_eq!(evaluation.findings.len(), 2);
}

/// A newly authored check the judge accepts is still run, in the configured
/// scratch site, before publication: one that crashes in its own code goes
/// back to its author with the crash.
#[cfg(unix)]
#[tokio::test]
async fn whole_set_freeze_returns_a_check_crashing_in_its_own_code_to_its_author() {
    use crate::command::workflow_task_set::executability::tests::{
        CRASHING, FIXED, configure_scratch,
    };
    let temp = tempfile::tempdir().unwrap();
    let (tasks, prd, _) = seed(&temp);
    let _scratch = configure_scratch(temp.path());
    let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut draft: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    draft["acceptance"][0]["check"]["command"] = serde_json::json!(CRASHING);
    std::fs::write(&path, serde_json::to_vec(&draft).unwrap()).unwrap();
    let client = Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, FIXED),
        |_, _| true,
    ));
    let scope = AuthorScope::for_task_set(temp.path(), &tasks, &prd)
        .expect("the task set's repository record is believed");
    let prepared = prepare_acceptance_freeze_reauthoring(
        temp.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        client.clone(),
        &scope,
    )
    .await
    .expect("the runnable re-author is accepted");
    assert_eq!(client.authored(), 1, "the crashing check was re-authored");
    assert!(
        client.prompts.lock().unwrap()[0].contains("lane() missing 1 required positional argument"),
        "the author saw the crash"
    );
    publish(temp.path(), prepared).expect("publishes");
    let contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(matches!(
        &contract.acceptance[0].check,
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } if command == FIXED
    ));
}
