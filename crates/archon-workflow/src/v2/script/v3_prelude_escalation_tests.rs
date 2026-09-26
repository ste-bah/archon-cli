//! Issue-107 in the prelude: a refused unit whose last verdict carries the
//! host's cross-owner plan gets exactly one extra round over the owners'
//! files; every other call is the one it was before.

use super::cross_task_tests::run_scripted;
use serde_json::{Value, json};

const SCRIPT: &str = r#"export const meta = { name: 'esc', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['src/a.rs'] },
  { id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: ['src/b.rs'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
const findings = [
  { id: 'gate', canonical_task_ids: ['TASK-A'], severity: 'high', claim: 'write path must fail closed' },
  { id: 'doc', canonical_task_ids: ['TASK-B'], severity: 'low', claim: 'stale doc' },
]
return await remediateFindings(findings, { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
"#;

fn landed() -> Value {
    json!({ "status": "accepted", "summary": "fixed", "items": [], "outcomes": [], "patch_landed": true,
        "result": { "status": "accepted", "summary": "fixed", "files_changed": [{"path": "src/a.rs"}],
            "commands_run": [{"command": "t", "status": "succeeded"}] } })
}

fn verdict(accept: bool, plan: bool) -> Value {
    let mut view = if accept {
        landed()
    } else {
        json!({ "status": "needs_review", "summary": "refused: must-pass tests red", "items": [], "outcomes": [],
            "result": { "status": "needs_review", "summary": "refused: must-pass tests red" } })
    };
    if plan {
        view["remediation_escalation"] = json!({
            "source": "host", "owner_task_ids": ["TASK-B"], "target_files": ["src/b_tests.rs"],
            "refutation": "refused: must-pass tests red",
            "blocker_evidence": [{"summary": "src/b_tests.rs calls the gated writer", "source": "src/b_tests.rs"}],
        });
    }
    view
}

/// Answers every call: fixes land, TASK-A's verdicts follow `a_verdicts` in
/// order, everything else accepts.
async fn run(a_verdicts: Vec<(bool, bool)>) -> (Vec<(String, Value)>, Value) {
    let queue = std::sync::Mutex::new(a_verdicts.into_iter());
    let (calls, result) = run_scripted(SCRIPT, move |method, payload| {
        let is_a = payload["options"]["remediationContract"]["taskId"] == "TASK-A";
        match method {
            "parallel" if is_a => {
                let (accept, plan) = queue
                    .lock()
                    .unwrap()
                    .next()
                    .expect("an A verdict per verify");
                verdict(accept, plan)
            }
            "parallel" => verdict(true, false),
            _ => landed(),
        }
    })
    .await;
    (calls, serde_json::from_str(&result).unwrap())
}

fn ids(calls: &[(String, Value)]) -> Vec<String> {
    calls
        .iter()
        .filter(|(method, _)| method != "checkpoint")
        .map(|(_, payload)| payload["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_blocker_in_another_tasks_file_buys_one_widened_round_that_decides_the_unit() {
    let (calls, result) = run(vec![(false, true), (false, true), (true, false)]).await;
    assert_eq!(
        ids(&calls),
        [
            "review-remediate-task-a-1-1",
            "verification-wave-review-verify-task-a-1-2",
            "review-remediate-task-a-2-3",
            "verification-wave-review-verify-task-a-2-4",
            "review-remediate-task-a-esc-5",
            "verification-wave-review-verify-task-a-esc-6",
            "review-remediate-task-b-1-7",
            "verification-wave-review-verify-task-b-1-8",
        ]
    );
    let fix = &calls[4].1;
    let contract = &fix["options"]["remediationContract"];
    assert_eq!(contract["round"], 3);
    assert_eq!(contract["maxRounds"], 2);
    assert_eq!(contract["taskId"], "TASK-A");
    assert_eq!(contract["escalation"]["ownerTaskIds"], json!(["TASK-B"]));
    assert!(
        contract.get("taskIds").is_none(),
        "the unit key is unchanged"
    );
    let item = &fix["source"][0];
    assert_eq!(item["canonical_task_ids"], json!(["TASK-A", "TASK-B"]));
    assert_eq!(item["target_files"], json!(["src/a.rs", "src/b_tests.rs"]));
    assert_eq!(item["escalation_owner_task_ids"], json!(["TASK-B"]));
    assert_eq!(item["escalation_blocker_paths"], json!(["src/b_tests.rs"]));
    let prompt = item["task"].as_str().unwrap();
    assert!(prompt.contains("ESCALATED cross-owner round"), "{prompt}");
    assert!(
        prompt.contains("write path must fail closed"),
        "original findings: {prompt}"
    );
    assert!(prompt.contains("PRIOR VERIFIER'S JUDGMENT"), "{prompt}");
    assert!(
        prompt.contains("src/b_tests.rs calls the gated writer"),
        "{prompt}"
    );
    let verify = &calls[5].1;
    assert_eq!(
        verify["source"][0]["canonical_task_ids"],
        json!(["TASK-A", "TASK-B"])
    );
    assert_eq!(verify["options"]["remediationContract"]["round"], 3);
    let check = verify["source"][0]["task"].as_str().unwrap();
    assert!(
        check.contains("Judge EVERY one of TASK-A, TASK-B"),
        "{check}"
    );
    assert!(check.contains("must-pass baseline tests"), "{check}");
    let resolved = result["resolved"].as_array().unwrap();
    let a = resolved
        .iter()
        .find(|e| e["taskId"] == "TASK-A")
        .expect("A resolved");
    assert_eq!(a["escalatedTo"], json!(["TASK-B"]));
}

#[tokio::test]
async fn without_a_plan_the_calls_are_exactly_what_they_were() {
    let (plain, plain_result) = run(vec![(false, false), (false, false)]).await;
    // A plan on round 1 alone does not change round 2, and a plan is only
    // spent once the regular rounds are.
    let (planned, _) = run(vec![(false, true), (false, true), (false, false)]).await;
    assert_eq!(
        ids(&plain),
        [
            "review-remediate-task-a-1-1",
            "verification-wave-review-verify-task-a-1-2",
            "review-remediate-task-a-2-3",
            "verification-wave-review-verify-task-a-2-4",
            "review-remediate-task-b-1-5",
            "verification-wave-review-verify-task-b-1-6",
        ]
    );
    assert_eq!(
        plain[..4],
        planned[..4],
        "round 1 and 2 calls are byte-identical"
    );
    let a = &plain_result["unresolved"][0];
    assert_eq!(a["taskId"], "TASK-A");
    assert_eq!(a["outcome"], "unverified");
    assert!(a.get("escalatedTo").is_none());
}

#[tokio::test]
async fn a_second_refusal_ends_the_unit_unverified_with_no_further_round() {
    let (calls, result) = run(vec![(false, true), (false, true), (false, true)]).await;
    let a_calls = ids(&calls)
        .into_iter()
        .filter(|id| id.contains("task-a"))
        .count();
    assert_eq!(
        a_calls, 6,
        "two regular rounds and one escalated: {calls:#?}"
    );
    let a = &result["unresolved"][0];
    assert_eq!(a["taskId"], "TASK-A");
    assert_eq!(a["outcome"], "unverified");
    assert_eq!(a["escalatedTo"], json!(["TASK-B"]));
}

#[tokio::test]
async fn an_escalated_round_that_lands_nothing_is_checkpointed_and_final() {
    let (calls, result) = run_scripted(SCRIPT, |method, payload| {
        let contract = &payload["options"]["remediationContract"];
        let escalated = contract.get("escalation").is_some();
        match method {
            "parallel" if contract["taskId"] == "TASK-A" => verdict(false, true),
            "fanout" if escalated => {
                let mut view = landed();
                view["patch_landed"] = json!(false);
                view
            }
            "parallel" => verdict(true, false),
            _ => landed(),
        }
    })
    .await;
    let checkpoint = calls
        .iter()
        .find(|(method, p)| {
            method == "checkpoint" && p["id"].as_str().unwrap().ends_with("-no-patch")
        })
        .expect("the escalated verify stage is checkpointed");
    assert_eq!(checkpoint.1["id"], "review-verify-task-a-3-no-patch");
    assert_eq!(
        checkpoint.1["options"]["remediationContract"]["escalation"]["ownerTaskIds"],
        json!(["TASK-B"])
    );
    let result: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["unresolved"][0]["taskId"], "TASK-A");
    let open = &result["unresolved"][0];
    assert_eq!(open["outcome"], "unverified", "{result}");
    assert!(
        open["reason"]
            .as_str()
            .unwrap()
            .contains("the last verifier's refusal stands: refused: must-pass tests red"),
        "{result}"
    );
    assert!(
        result["resolved"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["taskId"] != "TASK-A"),
        "{result}"
    );
}
