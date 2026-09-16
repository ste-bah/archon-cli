//! The fidelity section against a fake critic: both verdicts, the re-ask,
//! the cache, the unreachable provider, and the recorded waiver.
//!
//! Fixtures are a made-up PRD in a made-up domain, deliberately: the audit
//! must hold for any PRD, so nothing here may resemble a real one.

use std::sync::Mutex;

use archon_workflow::error::{WorkflowError, WorkflowResult};
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use async_trait::async_trait;

use super::*;

struct FakeCritic {
    replies: Mutex<Vec<Result<String, String>>>,
    prompts: Mutex<Vec<String>>,
}

impl FakeCritic {
    fn new(replies: Vec<Result<String, String>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies),
            prompts: Mutex::new(Vec::new()),
        })
    }
    fn calls(&self) -> usize {
        self.prompts.lock().unwrap().len()
    }
}

#[async_trait]
impl WorkflowLlmClient for FakeCritic {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the audit pins temperature 0.0")
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert_eq!(model, CRITIC_MODEL_ALIAS);
        assert_eq!(temperature, 0.0);
        let prompt = messages[0]["content"].as_str().unwrap().to_string();
        self.prompts.lock().unwrap().push(prompt);
        let mut replies = self.replies.lock().unwrap();
        assert!(!replies.is_empty(), "critic asked more often than scripted");
        match replies.remove(0) {
            Ok(content) => Ok(WorkflowAgentOutcome {
                content,
                stop_reason: Some("end_turn".into()),
                ..WorkflowAgentOutcome::default()
            }),
            Err(message) => Err(WorkflowError::port(std::io::Error::other(message))),
        }
    }
}

const LOOPHOLE: &str =
    "Focused tests run against a temporary target root; the shared store is untouched.";

/// One PRD with two obligations, one task claiming both. Returns the cwd.
fn corpus() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    let tasks = temp.path().join("tasks").join("PRD-WS-001");
    std::fs::create_dir_all(&tasks).expect("tasks dir");
    std::fs::write(
        temp.path().join("tasks").join("PRD-WS-001.md"),
        "## Goals\n\n| ID | Goal |\n|---|---|\n| G-WS-001 | Ingest widgets into the shared store. |\n\n## Acceptance Criteria\n\n| ID | Criterion |\n|---|---|\n| AC-WS-001 | Ingestion stores a registry entry. |\n",
    )
    .expect("prd");
    std::fs::write(
        tasks.join("TASK-WS-001.md"),
        format!(
            "# TASK-WS-001 — Ingest\n\n```yaml\ntask_id: TASK-WS-001\ntitle: Ingest\ncomplexity: medium\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"G-WS-001\", \"AC-WS-001\"]\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n## Scope\n\n{LOOPHOLE}\n\n## Focused Tests\n\n- `cargo test -p widgets`\n"
        ),
    )
    .expect("task");
    temp
}

fn reply(false_for: &[&str]) -> String {
    let verdicts = ["AC-WS-001", "G-WS-001"]
        .iter()
        .map(|id| {
            if false_for.contains(id) {
                serde_json::json!({"obligation_id": id, "necessarily_true": false, "weakest_task_id": "TASK-WS-001", "reason": "a temporary root leaves the shared store empty", "quoted_task_text": LOOPHOLE})
            } else {
                serde_json::json!({"obligation_id": id, "necessarily_true": true, "weakest_task_id": "", "reason": "the task must write the entry", "quoted_task_text": ""})
            }
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "verdicts": verdicts }).to_string()
}

async fn evaluate(cwd: &Path, client: Result<Arc<dyn WorkflowLlmClient>>) -> GateEvaluation {
    evaluate_lint_with_fidelity(
        cwd,
        &LintSource::Tasks(cwd.join("tasks").join("PRD-WS-001")),
        archon_core::config::GateMode::Enforce,
        client,
        &[],
    )
    .await
    .expect("evaluation")
}

#[tokio::test]
async fn a_false_verdict_blocks_with_body_scope_and_the_exact_finding_text() {
    let temp = corpus();
    let critic = FakeCritic::new(vec![Ok(reply(&["G-WS-001"]))]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(evaluation.operational_error().is_none());
    let finding = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.starts_with("obligation G-WS-001"))
        .expect("blocking fidelity finding");
    assert_eq!(
        finding.text,
        format!(
            "obligation G-WS-001 is claimed by TASK-WS-001 but none is obliged to make it true — a temporary root leaves the shared store empty — task TASK-WS-001: \"{LOOPHOLE}\""
        )
    );
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Body
    );
    assert_eq!(finding.subject, "G-WS-001");
    assert!(
        finding
            .source_path
            .as_ref()
            .unwrap()
            .ends_with("TASK-WS-001.md")
    );
    assert!(
        evaluation.report.contains("## obligation fidelity"),
        "{}",
        evaluation.report
    );
    assert!(evaluation.report.contains("BLOCKING obligation G-WS-001"));
    assert!(
        evaluation
            .report
            .contains("AC-WS-001: necessarily true given TASK-WS-001")
    );
    // Both obligations share one claimant, so the critic was asked once with both.
    assert_eq!(critic.calls(), 1);
    let prompt = &critic.prompts.lock().unwrap()[0];
    assert!(prompt.contains("\"id\":\"G-WS-001\"") && prompt.contains("\"id\":\"AC-WS-001\""));
    assert!(
        prompt.contains(LOOPHOLE),
        "the full task text travels with the question"
    );
}

#[tokio::test]
async fn a_true_verdict_adds_no_finding_and_is_served_from_cache_next_time() {
    let temp = corpus();
    let critic = FakeCritic::new(vec![Ok(reply(&[]))]);
    let first = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(
        !first
            .findings
            .iter()
            .any(|f| f.text.starts_with("obligation "))
    );
    assert!(first.report.contains("1 asked of"), "{}", first.report);
    let again = evaluate(temp.path(), Ok(critic.clone())).await;
    assert_eq!(critic.calls(), 1, "an unchanged set must not be re-asked");
    assert!(again.report.contains("0 asked of") && again.report.contains("1 served from"));
    // Editing the claiming task changes the digest and re-asks.
    let task = temp.path().join("tasks/PRD-WS-001/TASK-WS-001.md");
    std::fs::write(
        &task,
        std::fs::read_to_string(&task).unwrap() + "\nOne more allowance.\n",
    )
    .unwrap();
    let critic = FakeCritic::new(vec![Ok(reply(&[]))]);
    evaluate(temp.path(), Ok(critic.clone())).await;
    assert_eq!(critic.calls(), 1);
}

#[tokio::test]
async fn a_malformed_reply_is_re_asked_once_then_operational_never_a_pass() {
    let temp = corpus();
    let critic = FakeCritic::new(vec![Ok("not json".into()), Ok(r#"{"verdicts":[]}"#.into())]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert_eq!(critic.calls(), FIDELITY_ATTEMPTS);
    let error = evaluation
        .operational_error()
        .expect("operational, not a pass");
    assert!(
        error.contains("obligation fidelity audit failed operationally"),
        "{error}"
    );
    assert!(error.contains("after 2 attempts"), "{error}");
    let cache = temp.path().join(".archon/lint-cache/fidelity");
    let entries: Vec<_> = std::fs::read_dir(&cache)
        .expect("cache dir")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries,
        vec!["rejected"],
        "no verdict is cached; only the rejected replies are kept"
    );
    assert_eq!(
        std::fs::read_dir(cache.join("rejected")).unwrap().count(),
        FIDELITY_ATTEMPTS,
        "both raw replies are kept for diagnosis"
    );
    assert!(error.contains("reply kept at"), "{error}");

    let critic = FakeCritic::new(vec![Ok("```json\nnonsense\n```".into()), Ok(reply(&[]))]);
    let evaluation = evaluate(temp.path(), Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "corrected on the second ask"
    );
    assert_eq!(critic.calls(), 2);
}

#[tokio::test]
async fn an_unreachable_provider_is_operational_never_a_pass() {
    let temp = corpus();
    let evaluation = evaluate(temp.path(), Err(anyhow!("connection refused"))).await;
    let error = evaluation.operational_error().expect("operational");
    assert!(
        error.contains("connection refused") && error.contains("critic client"),
        "{error}"
    );
    let critic = FakeCritic::new(vec![Err("provider 503".into())]);
    let evaluation = evaluate(temp.path(), Ok(critic)).await;
    assert!(
        evaluation
            .operational_error()
            .unwrap()
            .contains("provider 503")
    );
}

#[tokio::test]
async fn waivers_are_recorded_verbatim_in_the_pin_and_downgrade_the_finding() {
    let temp = corpus();
    let cwd = temp.path();
    let tasks_root = cwd.join("tasks").join("PRD-WS-001");
    let waivers = waivers_from_flags(
        &["G-WS-001".into()],
        Some("  store lands in TASK-WS-002 next sprint "),
    )
    .expect("waiver");
    assert!(
        waivers_from_flags(&["G-WS-001".into()], Some("  ")).is_err(),
        "a reason is required"
    );
    assert!(waivers_from_flags(&[], None).unwrap().is_empty());
    let error = record_waivers(cwd, &tasks_root, &waivers).expect_err("no pin, no waiver");
    assert!(error.to_string().contains("freeze first"), "{error}");
    assert!(recorded_waivers(cwd, &tasks_root).is_empty());

    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(cwd, &tasks_root);
    std::fs::create_dir_all(pin_path.parent().unwrap()).unwrap();
    std::fs::write(&pin_path, serde_json::to_vec_pretty(&serde_json::json!({
        "task_root": tasks_root.canonicalize().unwrap(),
        "acceptance_digest": "d", "freeze_event_id": "e",
        "acceptance_gate": {"mode": "enforce", "finding_count": 0, "findings_digest": "f", "binary_commit": "b", "evaluated_at": "t"}
    })).unwrap()).unwrap();
    record_waivers(cwd, &tasks_root, &waivers).expect("recorded");
    let stamped = std::fs::read_to_string(&pin_path).unwrap();
    assert!(
        stamped.contains("\"reason\": \"store lands in TASK-WS-002 next sprint\""),
        "{stamped}"
    );
    assert!(stamped.contains("\"obligation_id\": \"G-WS-001\""));
    let recorded = recorded_waivers(cwd, &tasks_root);
    assert_eq!(recorded, waivers);
    // A second waiver for the same id replaces rather than accumulates.
    let newer = waivers_from_flags(&["G-WS-001".into()], Some("second thoughts")).unwrap();
    record_waivers(cwd, &tasks_root, &newer).unwrap();
    assert_eq!(recorded_waivers(cwd, &tasks_root), newer);

    let critic = FakeCritic::new(vec![Ok(reply(&["G-WS-001"]))]);
    let audit = audit(cwd, &tasks_root, critic.as_ref(), &newer)
        .await
        .expect("audit");
    assert!(
        audit.findings.is_empty(),
        "a waived false verdict does not block"
    );
    assert!(
        audit.report.contains("WAIVED obligation G-WS-001"),
        "{}",
        audit.report
    );
    assert!(audit.report.contains("by operator: \"second thoughts\""));
    let unwaived = audit_again(cwd, &tasks_root, critic.as_ref()).await;
    assert_eq!(
        unwaived.findings.len(),
        1,
        "without the waiver the same cached verdict blocks"
    );
}

async fn audit_again(
    cwd: &Path,
    tasks_root: &Path,
    client: &dyn WorkflowLlmClient,
) -> FidelityAudit {
    audit(cwd, tasks_root, client, &[]).await.expect("audit")
}

/// One task claiming nine obligations is asked twice, eight and then one,
/// each reply checked against exactly the ids its batch carried.
#[tokio::test]
async fn a_large_cluster_is_asked_in_bounded_batches() {
    let temp = corpus();
    let cwd = temp.path();
    let extra: Vec<String> = (1..=7).map(|n| format!("REQ-WS-{n:03}")).collect();
    let mut prd = std::fs::read_to_string(cwd.join("tasks/PRD-WS-001.md")).unwrap();
    prd.push_str("\n## Requirements\n\n");
    for id in &extra {
        prd.push_str(&format!("- {id}: requirement {id}\n"));
    }
    std::fs::write(cwd.join("tasks/PRD-WS-001.md"), prd).unwrap();
    let task = cwd.join("tasks/PRD-WS-001/TASK-WS-001.md");
    let body = std::fs::read_to_string(&task).unwrap().replace(
        "implements: [\"G-WS-001\", \"AC-WS-001\"]",
        &format!(
            "implements: [\"G-WS-001\", \"AC-WS-001\", {}]",
            extra
                .iter()
                .map(|id| format!("\"{id}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    std::fs::write(&task, body).unwrap();
    let true_verdict = |id: &str| serde_json::json!({"obligation_id": id, "necessarily_true": true, "reason": "obliged"});
    // Batches follow the sorted claim list: AC, G, REQ-001..006 then REQ-007.
    let first: Vec<_> = ["AC-WS-001", "G-WS-001"]
        .into_iter()
        .chain(extra[..6].iter().map(String::as_str))
        .map(true_verdict)
        .collect();
    let second = vec![true_verdict("REQ-WS-007")];
    let critic = FakeCritic::new(vec![
        Ok(serde_json::json!({ "verdicts": first }).to_string()),
        Ok(serde_json::json!({ "verdicts": second }).to_string()),
    ]);
    let evaluation = evaluate(cwd, Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(critic.calls(), 2);
    assert!(
        evaluation
            .report
            .contains("9 claimed obligation(s) in 1 cluster(s), 2 call batch(es)"),
        "{}",
        evaluation.report
    );
    assert!(
        evaluation
            .report
            .contains("REQ-WS-007: necessarily true given TASK-WS-001")
    );
    let prompts = critic.prompts.lock().unwrap();
    assert!(
        prompts[0].contains("\"id\":\"REQ-WS-006\"")
            && !prompts[0].contains("\"id\":\"REQ-WS-007\"")
    );
    assert!(
        prompts[1].contains("\"id\":\"REQ-WS-007\"")
            && !prompts[1].contains("\"id\":\"AC-WS-001\"")
    );
}

#[test]
fn the_decomposition_set_gate_carries_the_provider_and_the_body_scope() {
    let catalog = crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev")
        .expect("catalog");
    let gate = &catalog.capabilities["task-set-lint"];
    assert_eq!(
        gate.environment_profile,
        archon_workflow::EnvironmentProfileId::FreezeProvider
    );
    assert!(
        gate.remediation_scopes
            .contains(&archon_workflow::RemediationScope::Body)
    );
    assert!(gate.timeout_secs > crate::command::workflow_task_set::judge::JUDGE_TIMEOUT_SECS);
}
