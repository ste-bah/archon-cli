//! #549: every check is judged in its own call with its own named-file
//! budget; a check whose own files do not fit is not judged, never refuted.

use super::*;
use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use archon_workflow::task_set_contract::{
    AcceptanceCheck, GapPolicy, JudgeVerdict, PrdIdentity, TrustedCwd,
};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const BUDGET: usize = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;

/// A judge that answers every check it is shown, refuting one whose named
/// file it was not given (as the live judge did), and records its prompts.
#[derive(Default)]
struct Recording {
    prompts: Mutex<Vec<String>>,
    in_flight: AtomicUsize,
    most_in_flight: AtomicUsize,
    unusable_for: Option<&'static str>,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for Recording {
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
        _temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        let prompt = messages[0]["content"].as_str().unwrap().to_string();
        self.prompts.lock().unwrap().push(prompt.clone());
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_in_flight.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        let checks = checks_of(&prompt);
        let content = if self
            .unusable_for
            .is_some_and(|id| checks.iter().any(|check| check["id"] == id))
        {
            "I cannot decide.".to_string()
        } else {
            let decisions = checks
                .iter()
                .map(|check| {
                    let withheld = (check["named_files"].as_array().into_iter().flatten())
                        .any(|file| file["status"] != "supplied");
                    serde_json::json!({
                        "id": check["id"],
                        "verdict": if withheld { "refuted" } else { "accepted" },
                        "counterexample": if withheld { "the deciding inputs are not_supplied" } else { "none is constructible" },
                        "reason": if withheld { "the deciding test content is unknown" } else { "the named file decides it" },
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
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("the judge samples explicitly")
    }
}

fn checks_of(prompt: &str) -> Vec<serde_json::Value> {
    serde_json::from_str(prompt.rsplit_once("Checks: ").unwrap().1).unwrap()
}

fn entry(id: &str, file: &str) -> AcceptanceCriterion {
    AcceptanceCriterion {
        id: id.into(),
        criterion: format!("the behaviour {id} names holds"),
        check: AcceptanceCheck::Command {
            command: format!("operation {file}"),
            cwd: TrustedCwd::ProjectRoot,
        },
        gap_permitted: false,
        covers: Vec::new(),
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Refuted,
            counterexample: String::new(),
            reason: String::new(),
            host_call_id: String::new(),
            sampling: None,
        },
    }
}

/// A project holding `files` (name, size) and a contract whose checks each
/// name one of them, in order.
fn project(files: &[(&str, usize)]) -> (tempfile::TempDir, JudgeRoots, AcceptanceContract) {
    let root = tempfile::tempdir().unwrap();
    let mut acceptance = Vec::new();
    for (index, (name, size)) in files.iter().enumerate() {
        std::fs::write(root.path().join(name), "a".repeat(*size)).unwrap();
        acceptance.push(entry(&format!("AC-X-{:03}", index + 1), name));
    }
    let roots = JudgeRoots {
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    let contract = AcceptanceContract {
        schema_version: 1,
        prd: PrdIdentity {
            path: "p".into(),
            digest: "d".into(),
        },
        gap_policy: GapPolicy {
            permitted_acceptance_ids: Default::default(),
            forbidden_phrases: Vec::new(),
            required_fields: Vec::new(),
        },
        acceptance,
        supplementary: Vec::new(),
    };
    (root, roots, contract)
}

/// The one prompt that judged `id`; every prompt judges exactly one check.
fn prompt_for(client: &Recording, id: &str) -> String {
    let prompts = client.prompts.lock().unwrap();
    for prompt in prompts.iter() {
        assert_eq!(checks_of(prompt).len(), 1, "one check per call: {prompt}");
    }
    let mine = (prompts.iter())
        .filter(|prompt| checks_of(prompt)[0]["id"] == id)
        .collect::<Vec<_>>();
    assert_eq!(mine.len(), 1, "{id} is judged exactly once");
    mine[0].clone()
}

#[tokio::test]
async fn a_late_check_is_judged_with_its_own_named_files_549() {
    let (_root, roots, contract) =
        project(&[("first", BUDGET * 3 / 4), ("second", BUDGET * 3 / 4)]);
    let client = Recording::default();
    let judged = judge_entries_with_roots(&client, contract, "sonnet", &roots)
        .await
        .unwrap();
    let late = checks_of(&prompt_for(&client, "AC-X-002"));
    assert_eq!(late[0]["named_files"][0]["status"], "supplied");
    assert_eq!(
        late[0]["named_files"][0]["content"].as_str().unwrap().len(),
        BUDGET * 3 / 4
    );
    for entry in &judged.acceptance {
        assert_eq!(entry.judgment.verdict, JudgeDecision::Accepted, "{entry:?}");
    }
}

#[tokio::test]
async fn an_earlier_check_changes_no_later_prompt_or_reuse_key_549() {
    let (_root, roots, contract) = project(&[("early", BUDGET * 3 / 4), ("late", BUDGET * 3 / 4)]);
    let late_only = Recording::default();
    let mut alone = contract.clone();
    alone.acceptance.remove(0);
    judge_entries_with_roots(&late_only, alone.clone(), "sonnet", &roots)
        .await
        .unwrap();
    let both = Recording::default();
    judge_entries_with_roots(&both, contract.clone(), "sonnet", &roots)
        .await
        .unwrap();
    assert_eq!(
        prompt_for(&late_only, "AC-X-002"),
        prompt_for(&both, "AC-X-002")
    );
    let tasks = tempfile::tempdir().unwrap();
    let key = |entry: &AcceptanceCriterion| {
        crate::command::workflow_task_set::incremental::entry_key(
            &both,
            tasks.path(),
            "d",
            entry,
            &roots,
        )
    };
    let before = key(&alone.acceptance[0]);
    assert!(before.is_some());
    assert_eq!(before, key(&contract.acceptance[1]));
}

#[tokio::test]
async fn an_oversize_check_goes_to_its_author_unjudged_549() {
    let (_root, roots, contract) = project(&[("small", 64), ("huge", BUDGET + 1)]);
    let client = Recording::default();
    let judged = judge_entries_with_roots(&client, contract, "sonnet", &roots)
        .await
        .expect("no host hold and no pause: the check goes back to its author");
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    let oversize = &judged.acceptance[1];
    assert_ne!(oversize.judgment.verdict, JudgeDecision::Accepted);
    // The reason is the note its author gets (`author_note`).
    let note = &oversize.judgment.reason;
    assert_eq!(
        *note,
        format!(
            "check 'AC-X-002' names {} bytes of source (huge {} bytes), more than the judge can read ({BUDGET} bytes); name only the files whose content decides the criterion, or decide it with a command that does not need those files judged",
            BUDGET + 1,
            BUDGET + 1
        )
    );
    assert!(!note.contains("refuted"), "{note}");
    assert!(
        (client.prompts.lock().unwrap().iter()).all(|prompt| !prompt.contains("AC-X-002")),
        "the judge is never asked about a check whose files were withheld"
    );
}

#[tokio::test]
async fn parallel_calls_return_one_decision_per_id_549() {
    let files = ["a", "b", "c", "d", "e", "f"].map(|name| (name, 32));
    let (_root, roots, contract) = project(&files);
    let client = Recording::default();
    let judged = judge_entries_with_roots(&client, contract, "sonnet", &roots)
        .await
        .unwrap();
    let ids = (judged.acceptance.iter())
        .map(|entry| entry.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 6);
    for id in &ids {
        prompt_for(&client, id);
    }
    assert_eq!(client.prompts.lock().unwrap().len(), 6);
    for entry in &judged.acceptance {
        assert_eq!(entry.judgment.verdict, JudgeDecision::Accepted);
        assert_eq!(
            entry.judgment.host_call_id,
            format!("acceptance-judge-batch:{}", entry.id)
        );
    }
    let cap = crate::command::sona_workflow_shape_tuning::resolved_subagent_cap()
        .unwrap_or(1)
        .max(1);
    let most = client.most_in_flight.load(Ordering::SeqCst);
    assert!(most <= cap, "{most} calls at once, cap {cap}");
    if cap > 1 {
        assert!(most > 1, "the calls run concurrently");
    }
}

#[tokio::test]
async fn one_unusable_reply_holds_only_its_own_check_549() {
    let (_root, roots, contract) = project(&[("a", 8), ("b", 8), ("c", 8)]);
    let client = Recording {
        unusable_for: Some("AC-X-002"),
        ..Recording::default()
    };
    let error = judge_entries_with_roots(&client, contract, "sonnet", &roots)
        .await
        .expect_err("one call gave no usable verdict");
    let incomplete = JudgeIncomplete::caused(&error).expect("an incomplete judge");
    assert!(incomplete.is_authored_reply(), "{error:#}");
    let decided = (incomplete.decided.iter())
        .map(|entry| entry.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(decided, BTreeSet::from(["AC-X-001", "AC-X-003"]));
}

#[path = "workflow_task_set_judge_each_freeze_tests.rs"]
mod freeze_tests;
