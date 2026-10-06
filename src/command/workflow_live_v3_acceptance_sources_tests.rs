use std::sync::Mutex;

use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};

use super::*;

/// Replies with `content` and `stop`, recording each prompt and model.
struct Scripted {
    content: String,
    stop: Option<&'static str>,
    asked: Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for Scripted {
    /// Scripted replies stand for one continued session (#241).
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.run_agent(call).await
    }

    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the source judge samples explicitly")
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert_eq!(temperature, 0.0);
        let prompt = messages[0]["content"].as_str().unwrap().to_string();
        self.asked.lock().unwrap().push((prompt, model.to_string()));
        Ok(WorkflowAgentOutcome {
            content: self.content.clone(),
            stop_reason: self.stop.map(str::to_string),
            ..Default::default()
        })
    }
}

fn input() -> SourceJudgeInput {
    SourceJudgeInput {
        check_id: "AC-1".into(),
        criterion: "the value is 2".into(),
        command: "cargo test --test judge".into(),
        source: "tests/judge.rs".into(),
        origin: "landing".into(),
        pinned: Some("assert_eq!(value(), 2);".into()),
        pinned_retained: true,
        proposed: None,
        diff: None,
        part: None,
    }
}

#[tokio::test]
async fn the_source_judge_is_shown_both_versions_and_its_verdict_is_read() {
    let client = Scripted {
        content: "```json\n{\"verdict\":\"refuted\",\"counterexample\":\"value 1 passes\",\"reason\":\"deletes the only assertion\"}\n```".into(),
        stop: Some("end_turn"),
        asked: Mutex::new(Vec::new()),
    };
    let judge = LlmSourceJudge {
        client: &client,
        model: "judge-model".into(),
    };
    let verdict = judge.judge(&input()).await.unwrap();
    assert!(!verdict.accepted);
    assert_eq!(verdict.reason, "deletes the only assertion");
    let asked = client.asked.lock().unwrap();
    let (prompt, model) = &asked[0];
    assert_eq!(model, "judge-model");
    assert!(prompt.contains("assert_eq!(value(), 2);"), "{prompt}");
    assert!(
        prompt.contains("(the proposal deletes the source)"),
        "{prompt}"
    );
    assert!(
        prompt.contains("proposed by an implementing task's landing"),
        "{prompt}"
    );
}

#[tokio::test]
async fn a_truncated_or_unreadable_verdict_is_an_error_never_a_decision() {
    for (content, stop) in [
        (
            "{\"verdict\":\"accepted\",\"counterexample\":\"x\",\"reason\":\"y\"}",
            Some("max_tokens"),
        ),
        ("I think it is fine.", Some("end_turn")),
    ] {
        let client = Scripted {
            content: content.into(),
            stop,
            asked: Mutex::new(Vec::new()),
        };
        let judge = LlmSourceJudge {
            client: &client,
            model: "m".into(),
        };
        assert!(judge.judge(&input()).await.is_err(), "{content}");
    }
}

#[test]
fn the_freeze_time_judge_is_reused_and_there_is_no_default_model() {
    let mut contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path": "p", "digest": "d"}, "gap_policy": {},
        "acceptance": [{"id": "AC-1", "criterion": "c",
            "check": {"kind": "command", "command": "true", "cwd": "repo_root"},
            "judgment": {"verdict": "accepted", "counterexample": "n", "reason": "r", "host_call_id": "h"}}]
    }))
    .unwrap();
    assert!(
        recorded_judge(&contract).is_err(),
        "no recorded judge, no judging"
    );
    contract.acceptance[0].judgment.sampling =
        Some(serde_json::json!({"model": "frozen-judge", "provider": "p"}));
    assert_eq!(
        recorded_judge(&contract).unwrap(),
        ("frozen-judge".to_string(), "p".to_string())
    );
}

/// Review minor 10: pins nobody can read make every check a contract
/// defect for the round -- none still runs.
#[tokio::test]
async fn unreadable_pins_make_every_check_a_defect() {
    let dir = tempfile::tempdir().unwrap();
    let tasks = dir.path().join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        tasks.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        "not a contract",
    )
    .unwrap();
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path": "p", "digest": "d"}, "gap_policy": {},
        "acceptance": [
            {"id": "AC-1", "criterion": "c", "check": {"kind": "command", "command": "true", "cwd": "repo_root"},
             "judgment": {"verdict": "accepted", "counterexample": "n", "reason": "r", "host_call_id": "h"}},
            {"id": "AC-2", "criterion": "c", "check": {"kind": "command", "command": "true", "cwd": "repo_root"},
             "judgment": {"verdict": "accepted", "counterexample": "n", "reason": "r", "host_call_id": "h"}}]
    }))
    .unwrap();
    let context = StageContext {
        project: dir.path().to_path_buf(),
        task_root: tasks,
        repository: dir.path().to_path_buf(),
        binding: None,
        launch: None,
        launch_lineage: archon_workflow::task_set_lineage::LaunchLineage::Predates,
        run_id: "run".into(),
    };
    let mut record: AcceptanceRoundRecordV1 = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "run_id": "run", "call_id": "c", "round": 1, "attempt": 1,
        "max_rounds": 3, "contract_present": true, "final_round": false
    }))
    .unwrap();
    let outcome = apply(
        None,
        &context,
        &contract,
        &dir.path().join("run"),
        &mut record,
    )
    .await
    .expect("an unreadable pin is a defect, not a pause");
    assert_eq!(outcome.defects.len(), 2, "{:?}", outcome.defects);
    assert!(outcome.defects["AC-1"].contains("cannot be read"));
}

/// Review item 11: pending source changes settle BEFORE a repair
/// republishes the chain. A held change whose branch did not land is
/// orphaned first, so the in-round re-author of the same check can
/// republish; settled after it, the pending change would refuse it.
#[tokio::test]
async fn pending_source_changes_settle_before_the_repair_republishes() {
    use super::super::repair_tests::{run_fixture_with, stage};
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };
    let run = run_fixture_with(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", "bash scripts/two.sh", false),
    ]);
    super::super::repair_tests::record_baseline(&run);
    let project = run.set.project.path();
    std::fs::create_dir_all(project.join("scripts")).unwrap();
    std::fs::write(project.join("scripts/two.sh"), "exit 1\n").unwrap();
    let run_dir = run.store.run_dir(&run.run_id);
    let request = archon_workflow::check_source_requests::record(
        &run_dir,
        archon_workflow::check_source_requests::NewRequest {
            origin: archon_workflow::check_source_requests::ORIGIN_LANDING,
            check_ids: ["AC-F-002".to_string()].into(),
            root: archon_workflow::check_source_resolve::SourceRoot::Project,
            path: "scripts/two.sh",
            item: None,
            was_pinned: true,
            pinned_digest: None,
            proposed: Some(b"exit 0\n"),
            proposed_file: None,
            landed_file_digest: None,
            call_id: "wave",
            branch_id: "wave-0",
            task_ids: Vec::new(),
        },
    )
    .unwrap();
    // The proposing branch was refused: it did not land.
    archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2"))
        .save_branch_outcome(
            "wave",
            &archon_workflow::WorkflowV2BranchOutcome {
                item_id: "wave-0".into(),
                role: "coder".into(),
                status: archon_workflow::WorkflowV2Status::NeedsReview,
                result: None,
                error: None,
                failure_kind: None,
                item_input_hash: None,
                completion_evidence: Vec::new(),
            },
        )
        .unwrap();
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f present && test -s present"),
        |_, _| true,
    );
    let (_, record) = stage(&run, &client).await;
    let source = record
        .contract_repairs
        .iter()
        .find(|repair| repair.trigger == REPAIR_TRIGGER_CHECK_SOURCE)
        .unwrap_or_else(|| panic!("{:?}", record.contract_repairs));
    assert!(
        source.diagnostics[0].contains(&request.request_id),
        "{source:?}"
    );
    assert!(source.failure.starts_with("orphaned"), "{source:?}");
    let reauthored = record
        .contract_repairs
        .iter()
        .find(|repair| {
            repair.check_ids == ["AC-F-002"] && repair.trigger != REPAIR_TRIGGER_CHECK_SOURCE
        })
        .unwrap_or_else(|| panic!("{:?}", record.contract_repairs));
    assert!(reauthored.repaired, "{reauthored:?}");
}
