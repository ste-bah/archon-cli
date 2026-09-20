//! The acceptance stage (Obs-32): pre-flight ordering, the schema marker, and
//! the prelude loop's routing, driven through QuickJS against a scripted host.

use super::*;
use std::sync::{Arc, Mutex as StdMutex};

const TASK_IDS: [&str; 2] = ["TASK-Q-001", "TASK-Q-002"];

fn expected() -> std::collections::BTreeSet<String> {
    TASK_IDS.iter().map(|id| id.to_string()).collect()
}

/// A complete v3 script: two tasks, both reviews, review remediation, then
/// whatever `tail` says, so each test states only the ending it is about.
fn script(schema: &str, tail: &str) -> String {
    format!(
        r#"export const meta = {{ name: 'accept', description: 'd', {schema}phases: [] }}
const tasks = [
  {{ id: 'TASK-Q-001', file: 'tasks/TASK-Q-001.md', targetFiles: ['src/one.txt'] }},
  {{ id: 'TASK-Q-002', file: 'tasks/TASK-Q-002.md', targetFiles: ['src/two.txt'] }},
]
const byId = (id) => tasks.find((t) => t.id === id)
const accepted_ids = []
for (const t of tasks) {{
  const impl = await agent(`Implement ${{t.id}}`, {{ label: `implement-${{t.id.toLowerCase()}}`, write: true, taskIds: [t.id], targetFiles: t.targetFiles }})
  const check = await agent(`Verify ${{t.id}}`, {{ label: `verify-${{t.id.toLowerCase()}}`, verify: true, taskIds: [t.id] }})
  if (usable(impl) && accepted(check)) accepted_ids.push(t.id)
}}
const adversarial_findings = await adversarialReview(accepted_ids, {{ evidenceFor: () => [] }})
const uncovered_requirements = await coverageAudit(accepted_ids, {{ evidenceFor: () => [] }})
const review_remediation = await remediateFindings([...adversarial_findings, ...uncovered_requirements], {{ taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles }})
{tail}
"#
    )
}

const ACCEPTANCE_TAIL: &str = r#"const acceptance_gate = await acceptance({ taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
return { accepted: accepted_ids, blocked: [], adversarial_findings, uncovered_requirements, review_remediation, acceptance_gate, notes: 'n' }"#;

const NO_ACCEPTANCE_TAIL: &str = r#"return { accepted: accepted_ids, blocked: [], adversarial_findings, uncovered_requirements, review_remediation, notes: 'n' }"#;

#[test]
fn the_schema_marker_is_read_from_the_meta_declaration_only() {
    assert_eq!(authored_script_schema(&script("schema: 2, ", "")), Some(2));
    assert_eq!(authored_script_schema(&script("", "")), None);
    // A `schema:` mention outside `meta` is not the marker.
    let elsewhere = format!("{}\nconst other = {{ schema: 2 }}\n", script("", ""));
    assert_eq!(authored_script_schema(&elsewhere), None);
    assert!(requires_acceptance_stage(&script("schema: 2, ", "")));
    assert!(!requires_acceptance_stage(&script("schema: 1, ", "")));
    assert!(schema_marker_defect(&script("", "")).is_some());
    assert!(schema_marker_defect(&script("schema: 2, ", "")).is_none());
}

/// A fresh draft without the acceptance stage is refused with a message that
/// names the call to add; one with it passes. Both drafts carry the marker.
#[tokio::test]
async fn a_new_draft_must_end_with_the_acceptance_stage() {
    let error = validate_authored_draft(&script("schema: 2, ", NO_ACCEPTANCE_TAIL), &expected())
        .await
        .expect_err("no acceptance stage");
    assert!(error.contains("no acceptance stage"), "{error}");
    assert!(error.contains("await acceptance("), "{error}");
    validate_authored_draft(&script("schema: 2, ", ACCEPTANCE_TAIL), &expected())
        .await
        .expect("the acceptance-ending draft passes");
}

/// A fresh draft that omits the marker is refused even when it ends with
/// the stage: the marker is what binds persisted scripts to the rule.
#[tokio::test]
async fn a_new_draft_must_carry_the_schema_marker() {
    let error = validate_authored_draft(&script("", ACCEPTANCE_TAIL), &expected())
        .await
        .expect_err("missing marker");
    assert!(error.contains("schema: 2"), "{error}");
}

/// Persisted scripts of older runs (no marker) keep passing the plan
/// pre-flight without the stage, so resuming them still works.
#[tokio::test]
async fn a_persisted_script_without_the_marker_is_not_held_to_the_stage() {
    validate_authored_plan(&script("", NO_ACCEPTANCE_TAIL), &expected())
        .await
        .expect("legacy persisted script resumes");
    let error = validate_authored_plan(&script("schema: 2, ", NO_ACCEPTANCE_TAIL), &expected())
        .await
        .expect_err("a marked script is held to the stage on resume too");
    assert!(error.contains("no acceptance stage"), "{error}");
}

/// Work after the stage, or the stage before the reviews, is a defect.
#[tokio::test]
async fn the_acceptance_stage_must_be_last_and_after_the_reviews() {
    let work_after = ACCEPTANCE_TAIL.replace(
        "return {",
        "await agent('late', { label: 'late-work', write: true, taskIds: ['TASK-Q-001'], targetFiles: ['src/one.txt'] })\nreturn {",
    );
    let error = validate_authored_draft(&script("schema: 2, ", &work_after), &expected())
        .await
        .expect_err("work after acceptance");
    assert!(
        error.contains("AFTER the final acceptance round"),
        "{error}"
    );

    let early = script("schema: 2, ", NO_ACCEPTANCE_TAIL).replace(
        "const adversarial_findings",
        "const acceptance_gate = await acceptance({ taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })\nconst adversarial_findings",
    );
    let error = validate_authored_draft(&early, &expected())
        .await
        .expect_err("acceptance before reviews");
    assert!(error.contains("BEFORE the final review reduce"), "{error}");
}

#[tokio::test]
async fn the_dry_run_plans_exactly_one_clean_round_as_the_last_call() {
    let details = dry_run_workflow_plan_full_details(&script("schema: 2, ", ACCEPTANCE_TAIL), None)
        .await
        .expect("plan");
    let rounds: Vec<&WorkflowV2HostCall> = details
        .calls
        .iter()
        .filter(|call| is_acceptance_stage_call(call))
        .collect();
    assert_eq!(rounds.len(), 1, "{:?}", details.calls);
    assert_eq!(rounds[0].id, "acceptance-contract-run-1");
    assert_eq!(rounds[0].options.extra["checkIds"], serde_json::json!([]));
    assert!(std::ptr::eq(
        rounds[0],
        details.calls.last().expect("calls")
    ));
    assert!(acceptance_stage_defects(&details.calls).is_empty());
    validate_executed_acceptance_stage(&script("schema: 2, ", ACCEPTANCE_TAIL), &details.calls)
        .expect("the executed sequence ends with the stage");
}

/// Run a v3 script against a scripted host: every host call is recorded and
/// answered by `answer(method, payload)`.
async fn run_scripted(
    source: &str,
    answer: impl Fn(&str, &serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
) -> (Vec<(String, serde_json::Value)>, String) {
    use rquickjs::function::{Async, Func};
    use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
    let calls: Arc<StdMutex<Vec<(String, serde_json::Value)>>> = Default::default();
    let answer = Arc::new(answer);
    let source = script_source(source, None);
    let runtime = AsyncRuntime::new().expect("runtime");
    let context = AsyncContext::full(&runtime).await.expect("context");
    let calls_for_js = calls.clone();
    let result = context
        .async_with(async move |ctx| {
            ctx.globals()
                .set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let calls = calls_for_js.clone();
                        let answer = answer.clone();
                        async move {
                            let payload: serde_json::Value =
                                serde_json::from_str(&payload).expect("payload json");
                            calls
                                .lock()
                                .unwrap()
                                .push((method.clone(), payload.clone()));
                            Ok::<_, rquickjs::Error>(answer(&method, &payload).to_string())
                        }
                    })),
                )
                .expect("bind host");
            let promise: Promise = ctx
                .eval(source.as_str())
                .catch(&ctx)
                .map_err(|e| e.to_string())?;
            promise
                .into_future::<String>()
                .await
                .catch(&ctx)
                .map_err(|e| e.to_string())
        })
        .await
        .expect("script completes");
    let calls = calls.lock().unwrap().clone();
    (calls, result)
}

/// A stub result view in the live shape: `data` keys spread at the top level.
fn view(mut data: serde_json::Value, status: &str) -> serde_json::Value {
    let object = data.as_object_mut().expect("object");
    object.insert("status".into(), serde_json::json!(status));
    object.insert("summary".into(), serde_json::json!("stub"));
    object.insert(
        "result".into(),
        serde_json::json!({ "status": status, "summary": "stub", "files_changed": [{"path": "x"}], "commands_run": [{"command": "c", "status": "succeeded"}] }),
    );
    data
}

fn acceptance_reply(
    round: u64,
    failing: serde_json::Value,
    final_round: bool,
) -> serde_json::Value {
    view(
        serde_json::json!({
            "round": round,
            "final": final_round,
            "failing": failing,
            "passed": [],
            "contract_present": true,
            "operational_errors": [],
            "record_path": format!("v2/acceptance/round-0{round}/attempt-01.json"),
        }),
        "accepted",
    )
}

/// Failing checks are routed to the owning tasks through remediateFindings,
/// an unowned failing check is reported and not remediated, and the next
/// round asks for ONLY the checks that failed.
#[tokio::test]
async fn failing_checks_route_to_owning_tasks_and_only_they_re_run() {
    let (calls, result) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |method, payload| {
        let id = payload["id"].as_str().unwrap_or_default();
        if id == "acceptance-contract-run-1" {
            return acceptance_reply(
                1,
                serde_json::json!([
                    { "check_id": "REQ-2", "criterion": "two is done", "kind": "command", "exit_code": 1, "owning_tasks": ["TASK-Q-002"], "stderr_tail": "boom" },
                    { "check_id": "REQ-9", "criterion": "set-level", "kind": "command", "exit_code": 1, "owning_tasks": [] }
                ]),
                false,
            );
        }
        if id == "acceptance-contract-run-2" {
            return acceptance_reply(
                2,
                serde_json::json!([{ "check_id": "REQ-9", "criterion": "set-level", "kind": "command", "exit_code": 1, "owning_tasks": [] }]),
                true,
            );
        }
        let _ = method;
        view(serde_json::json!({ "items": [], "outcomes": [] }), "accepted")
    })
    .await;
    let acceptance_calls: Vec<&(String, serde_json::Value)> = calls
        .iter()
        .filter(|(_, payload)| {
            payload["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("acceptance-contract-run-"))
        })
        .collect();
    assert_eq!(acceptance_calls.len(), 2, "{calls:#?}");
    assert_eq!(acceptance_calls[0].0, "tool");
    assert_eq!(
        acceptance_calls[0].1["options"]["tool"],
        "acceptance-contract-run"
    );
    assert_eq!(
        acceptance_calls[0].1["options"]["checkIds"],
        serde_json::json!([])
    );
    assert_eq!(
        acceptance_calls[1].1["options"]["checkIds"],
        serde_json::json!(["REQ-2", "REQ-9"]),
        "round 2 re-runs only what failed"
    );
    // Between the rounds: a remediation write for the owning task, then its verifier.
    let first = calls
        .iter()
        .position(|(_, p)| p["id"] == "acceptance-contract-run-1")
        .unwrap();
    let second = calls
        .iter()
        .position(|(_, p)| p["id"] == "acceptance-contract-run-2")
        .unwrap();
    let between: Vec<&(String, serde_json::Value)> = calls[first + 1..second].iter().collect();
    let remediate = between
        .iter()
        .find(|(method, p)| {
            method == "fanout"
                && p["id"]
                    .as_str()
                    .unwrap()
                    .contains("review-remediate-task-q-002")
        })
        .unwrap_or_else(|| panic!("a remediation write for TASK-Q-002: {between:#?}"));
    let item = &remediate.1["source"][0];
    assert_eq!(
        item["canonical_task_ids"],
        serde_json::json!(["TASK-Q-002"])
    );
    assert_eq!(item["target_files"], serde_json::json!(["src/two.txt"]));
    let prompt = item["task"].as_str().unwrap();
    assert!(
        prompt.contains("REQ-2") && prompt.contains("boom"),
        "{prompt}"
    );
    assert!(
        !prompt.contains("REQ-9"),
        "the unowned check is not sent to a task: {prompt}"
    );
    assert_eq!(
        remediate.1["options"]["remediationContract"]["taskId"],
        "TASK-Q-002"
    );
    assert!(
        between.iter().any(|(m, p)| m == "parallel"
            && p["id"]
                .as_str()
                .unwrap()
                .contains("review-verify-task-q-002")),
        "the fix is re-verified: {between:#?}"
    );
    assert!(
        !between
            .iter()
            .any(|(_, p)| p["id"].as_str().unwrap().contains("task-q-001")),
        "TASK-Q-001 owns no failing check and is left alone: {between:#?}"
    );
    let result: serde_json::Value = serde_json::from_str(&result).expect("accounting json");
    let gate = &result["acceptance_gate"];
    assert_eq!(gate["complete"], false);
    assert_eq!(gate["rounds"].as_array().unwrap().len(), 2);
    assert_eq!(gate["unowned_failing"][0]["check_id"], "REQ-9");
    assert_eq!(gate["failing"].as_array().unwrap().len(), 1);
}

/// A clean first round ends the stage: no remediation, no second round.
#[tokio::test]
async fn a_clean_round_ends_the_stage_without_remediation() {
    let (calls, result) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |_, payload| {
        if payload["id"] == "acceptance-contract-run-1" {
            return acceptance_reply(1, serde_json::json!([]), true);
        }
        view(
            serde_json::json!({ "items": [], "outcomes": [] }),
            "accepted",
        )
    })
    .await;
    let rounds = calls
        .iter()
        .filter(|(_, p)| {
            p["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("acceptance-contract-run-"))
        })
        .count();
    assert_eq!(rounds, 1);
    assert!(std::ptr::eq(
        calls.iter().rev().find(|(m, _)| m != "checkpoint").unwrap(),
        calls
            .iter()
            .find(|(_, p)| p["id"] == "acceptance-contract-run-1")
            .unwrap()
    ));
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["acceptance_gate"]["complete"], true);
}

/// The host's `final` flag is followed even while checks still fail: the
/// last permitted round is not followed by another.
#[tokio::test]
async fn the_host_final_flag_stops_the_loop_with_failures_left() {
    let (calls, result) = run_scripted(&script("schema: 2, ", ACCEPTANCE_TAIL), |_, payload| {
        if payload["id"] == "acceptance-contract-run-1" {
            return acceptance_reply(
                1,
                serde_json::json!([{ "check_id": "REQ-1", "criterion": "c", "kind": "command", "exit_code": 1, "owning_tasks": ["TASK-Q-001"] }]),
                true,
            );
        }
        view(serde_json::json!({ "items": [], "outcomes": [] }), "accepted")
    })
    .await;
    assert!(
        !calls
            .iter()
            .any(|(_, p)| p["id"] == "acceptance-contract-run-2"),
        "a final round is not followed by another: {calls:#?}"
    );
    assert!(
        !calls
            .iter()
            .any(|(_, p)| p["id"].as_str().unwrap().contains("review-remediate")),
        "no remediation after the final round: {calls:#?}"
    );
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["acceptance_gate"]["complete"], false);
    assert_eq!(result["acceptance_gate"]["failing"][0]["check_id"], "REQ-1");
}
