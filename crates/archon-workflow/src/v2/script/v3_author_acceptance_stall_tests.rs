//! Issue 288: the acceptance loop's stall belt compares a round with the last
//! round that actually ran checks, and a stall pauses the run; it never ends
//! the loop while the host keeps it open.
use super::tests::{ACCEPTANCE_TAIL, acceptance_reply, run_scripted, script, view};

fn ids(calls: &[(String, serde_json::Value)], method: &str) -> Vec<String> {
    (calls.iter())
        .filter(|(m, _)| m == method)
        .filter_map(|(_, p)| p["id"].as_str().map(str::to_string))
        .collect()
}

fn rounds(calls: &[(String, serde_json::Value)]) -> Vec<String> {
    (calls.iter())
        .filter_map(|(_, p)| p["id"].as_str())
        .filter(|id| id.starts_with("acceptance-contract-run-"))
        .map(str::to_string)
        .collect()
}

/// Two rounds that ran checks fail the same check and nothing is sent
/// between them (no unit owns it; the host repairs it itself). The loop used
/// to END after the second round while the host kept it open. Now the run
/// pauses on the stall, and the resumed run goes on with the loop.
#[tokio::test]
async fn a_repeat_after_a_round_that_sent_nothing_pauses_and_the_loop_goes_on() {
    let unowned = serde_json::json!([{ "check_id": "REQ-9", "criterion": "set-level", "kind": "command",
        "status": "failed", "exit_code": 1, "owning_tasks": [] }]);
    let (calls, result) = run_scripted(
        &script("schema: 2, ", ACCEPTANCE_TAIL),
        move |method, payload| {
            let id = payload["id"].as_str().unwrap_or_default();
            if method == "pause" {
                return serde_json::json!({ "resumed": true, "pause_id": id });
            }
            match id {
                "acceptance-contract-run-1" => acceptance_reply(1, unowned.clone(), false),
                "acceptance-contract-run-2" => acceptance_reply(2, unowned.clone(), false),
                id if id.starts_with("acceptance-contract-run-") => {
                    acceptance_reply(3, serde_json::json!([]), true)
                }
                _ => view(
                    serde_json::json!({ "items": [], "outcomes": [] }),
                    "accepted",
                ),
            }
        },
    )
    .await;
    assert_eq!(ids(&calls, "pause"), ["acceptance-stall-2"], "{calls:#?}");
    let pause = calls.iter().find(|(m, _)| m == "pause").unwrap();
    let evidence = &pause.1["options"]["evidence"];
    assert_eq!(evidence["reason"], "no_progress");
    assert_eq!(evidence["compared_with_round"], 1);
    assert_eq!(evidence["failing_check_ids"], serde_json::json!(["REQ-9"]));
    assert_eq!(
        rounds(&calls),
        [
            "acceptance-contract-run-1",
            "acceptance-contract-run-2",
            "acceptance-contract-run-3"
        ],
        "the loop goes on after the pause"
    );
    let result: serde_json::Value = serde_json::from_str(&result).expect("accounting json");
    assert_eq!(result["acceptance_gate"]["complete"], true);
}

/// A round that evaluated no check (every failing check is an `error`) is
/// never the round a repeat is judged against: after a round that sent a fix,
/// two such rounds in a row used to end the loop. Now the loop runs until the
/// host ends it, with no pause.
#[tokio::test]
async fn rounds_that_ran_no_checks_never_end_the_loop() {
    let (calls, result) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |method, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        if method == "pause" {
            return serde_json::json!({ "resumed": true, "pause_id": id });
        }
        let erroring = serde_json::json!([{ "check_id": "REQ-1", "criterion": "one", "kind": "command",
            "status": "error", "exit_code": null, "operational_error": "scratch could not be built",
            "owning_tasks": ["TASK-Q-001"] }]);
        match id {
            "acceptance-contract-run-1" => acceptance_reply(1, serde_json::json!([{ "check_id": "REQ-1", "criterion": "one",
                "kind": "command", "status": "failed", "exit_code": 1, "owning_tasks": ["TASK-Q-001"] }]), false),
            "acceptance-contract-run-2" => acceptance_reply(2, erroring, false),
            "acceptance-contract-run-3" => acceptance_reply(3, erroring, false),
            id if id.starts_with("acceptance-contract-run-") => acceptance_reply(4, serde_json::json!([]), true),
            _ => view(serde_json::json!({ "items": [], "outcomes": [] }), "accepted"),
        }
    })
    .await;
    assert!(ids(&calls, "pause").is_empty(), "{calls:#?}");
    assert_eq!(rounds(&calls).len(), 4, "{:?}", rounds(&calls));
    let result: serde_json::Value = serde_json::from_str(&result).expect("accounting json");
    assert_eq!(result["acceptance_gate"]["complete"], true);
}
