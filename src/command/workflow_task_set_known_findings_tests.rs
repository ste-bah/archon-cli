//! #552 review: a check the judge could not decide is known before the later
//! stages of the freeze run. When a later stage ends incomplete, that check's
//! own finding still reaches its author in the same round, and an unchanged
//! check is not judged again.

use super::*;

/// Accepts every check in the acceptance judge except [`UNDECIDED`], whose
/// replies are never usable; every passability batch (checks shown with
/// their baseline output) is unusable too. Counts the acceptance-judge asks
/// about [`UNDECIDED`].
#[derive(Default)]
struct UndecidedJudge {
    undecided_asks: AtomicUsize,
}

const UNDECIDED: &str = "AC-X-002";

#[async_trait]
impl WorkflowLlmClient for UndecidedJudge {
    fn provider_id(&self) -> Option<String> {
        Some("scripted".into())
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<Value>,
        _system: Vec<Value>,
        _tools: Vec<Value>,
        _model: &str,
        _temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        let prompt = messages[0]["content"].as_str().expect("prompt").to_string();
        let checks: Vec<Value> = serde_json::from_str(
            prompt
                .split_once("Checks: ")
                .expect("checks")
                .1
                .split("\n\n")
                .next()
                .unwrap(),
        )
        .expect("checks are JSON");
        let passability = checks.iter().any(|check| check.get("baseline").is_some());
        let undecided = checks.iter().any(|check| check["id"] == UNDECIDED);
        if undecided && !passability {
            self.undecided_asks.fetch_add(1, SeqCst);
        }
        let content = if passability || undecided {
            "no json".to_string()
        } else {
            let decisions = (checks.iter())
                .map(|check| {
                    serde_json::json!({
                        "id": check["id"], "verdict": "accepted",
                        "counterexample": "none is constructible",
                        "reason": "the failure is the absent feature",
                    })
                })
                .collect::<Vec<_>>();
            serde_json::json!({ "decisions": decisions }).to_string()
        };
        Ok(WorkflowAgentOutcome {
            content,
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }

    async fn send_message(
        &self,
        messages: Vec<Value>,
        system: Vec<Value>,
        tools: Vec<Value>,
        model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.send_message_with_temperature(messages, system, tools, model, 0.0)
            .await
    }
}

/// `outside_set` with a second criterion and check, [`UNDECIDED`], that also
/// fails before any implementation.
fn two_checks() -> (tempfile::TempDir, tempfile::TempDir, PathBuf, PathBuf) {
    let (project, outside, tasks, prd) = outside_set(ABSENT);
    std::fs::write(
        &prd,
        "## Acceptance Criteria\n| ID | Criterion |\n|---|---|\n| AC-X-001 | output is valid |\n| AC-X-002 | other output is valid |\n",
    )
    .unwrap();
    let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut draft: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut second = draft["acceptance"][0].clone();
    second["id"] = UNDECIDED.into();
    second["check"]["command"] =
        "python3 -c 'import json; assert json.load(open(\"other.json\"))[\"valid\"]'".into();
    draft["acceptance"].as_array_mut().unwrap().push(second);
    std::fs::write(&path, serde_json::to_vec(&draft).unwrap()).unwrap();
    (project, outside, tasks, prd)
}

async fn staged_freeze(
    project: &Path,
    tasks: &Path,
    prd: &Path,
    client: &Arc<UndecidedJudge>,
) -> anyhow::Error {
    let candidate = std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    prepare_acceptance_freeze_resumable(
        project,
        tasks,
        prd,
        GateMode::Observe,
        candidate,
        client.clone(),
        &saving(),
    )
    .await
    .expect_err("the passability judge of the other check cannot complete")
}

#[tokio::test]
async fn an_undecided_check_reaches_its_author_when_a_sibling_passability_judge_is_incomplete_552()
{
    let (project, _outside, tasks, prd) = two_checks();
    let client = Arc::new(UndecidedJudge::default());
    let error = staged_freeze(project.path(), &tasks, &prd, &client).await;
    let incomplete = crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error)
        .unwrap_or_else(|| panic!("an incomplete freeze: {error:#}"));
    assert!(incomplete.is_authored_judge_reply(), "{error:#}");
    let envelope =
        crate::command::workflow::workflow_staged_cli::task_set_error::gate_error_evaluation(
            &error,
        )
        .into_envelope()
        .unwrap();
    let operational = envelope
        .operational_error
        .clone()
        .expect("an operational error");
    assert_eq!(operational.kind, "authored_judge_reply");
    assert!(
        operational.subjects.contains(&"AC-X-001".to_string())
            && operational.subjects.contains(&UNDECIDED.to_string()),
        "both checks re-author: {:?}",
        operational.subjects
    );
    let own = (envelope.policy_findings.iter())
        .find(|finding| finding.subject == UNDECIDED)
        .unwrap_or_else(|| panic!("the undecided check's own note: {envelope:?}"));
    assert_eq!(
        own.remediation_scope,
        archon_workflow::RemediationScope::CandidateArtifact
    );
    assert!(
        own.text
            .starts_with("the judge returned no usable verdict for check 'AC-X-002': "),
        "{}",
        own.text
    );
}

#[tokio::test]
async fn an_unchanged_undecided_check_is_not_judged_again_552() {
    let (project, _outside, tasks, prd) = two_checks();
    let client = Arc::new(UndecidedJudge::default());
    staged_freeze(project.path(), &tasks, &prd, &client).await;
    let first = client.undecided_asks.load(SeqCst);
    assert!(first > 0, "the first freeze asks the judge");
    let error = staged_freeze(project.path(), &tasks, &prd, &client).await;
    assert_eq!(
        client.undecided_asks.load(SeqCst),
        first,
        "the unchanged check reuses its saved outcome"
    );
    let envelope =
        crate::command::workflow::workflow_staged_cli::task_set_error::gate_error_evaluation(
            &error,
        )
        .into_envelope()
        .unwrap();
    assert!(
        (envelope.policy_findings.iter()).any(|finding| finding.subject == UNDECIDED),
        "the reused outcome still reaches its author: {envelope:?}"
    );
}
