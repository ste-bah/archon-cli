//! REM-14, prelude level: the completion units the host plans, run by the
//! real prelude against a fake host that answers every call and records it.

use std::sync::{Arc, Mutex as StdMutex};

use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};

use crate::v2::script::script_source;
use crate::v2::script::task_completion::{COMPLETION_MODE_NOOP_VERIFY, completion_unit};

/// The script skips TASK-N, runs one review, and never remediates.
const SCRIPT: &str = "export const meta = { name: 'c', description: 'd', phases: [] }\nawait adversarialReview([])\nreturn { accepted: [], blocked: [] }\n";

/// Run `SCRIPT` with the host answering the completion plan with one
/// no-op-verify unit and its verifier with `verdict`; the ids the prelude
/// called, in order, and the accounting it returned.
async fn run(verdict: &'static str) -> (Vec<String>, serde_json::Value) {
    let calls: Arc<StdMutex<Vec<String>>> = Default::default();
    let recorded = calls.clone();
    let unit = completion_unit("TASK-N");
    let source = script_source(SCRIPT, None);
    let runtime = AsyncRuntime::new().expect("runtime");
    let context = AsyncContext::full(&runtime).await.expect("context");
    let out: String = context
        .async_with(async move |ctx| {
            ctx.globals()
                .set(
                    "__archonHost",
                    Func::from(Async(move |_method: String, payload: String| {
                        let recorded = recorded.clone();
                        let unit = unit.clone();
                        async move {
                            let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
                            let id = payload["id"].as_str().unwrap_or_default().to_string();
                            recorded.lock().unwrap().push(id.clone());
                            let answer = if id == "task-completion" {
                                serde_json::json!({ "status": "accepted", "data": { "task_completion": [{
                                    "source": "host", "task_id": "TASK-N", "unit": unit,
                                    "task_file": "tasks/TASK-N.md", "target_files": [], "focused_tests": [],
                                    "artifacts": [], "mode": COMPLETION_MODE_NOOP_VERIFY }] } })
                            } else if id.starts_with("verification-wave-") {
                                serde_json::json!({ "status": verdict, "summary": format!("verifier said {verdict}") })
                            } else {
                                serde_json::json!({ "status": "accepted", "summary": "stub", "final": true,
                                    "failing": [], "review_findings": { "findings": [] } })
                            };
                            Ok::<_, rquickjs::Error>(answer.to_string())
                        }
                    })),
                )
                .expect("bind host");
            let promise: Promise = ctx.eval(source.as_str()).catch(&ctx).map_err(|e| e.to_string())?;
            promise.into_future::<String>().await.catch(&ctx).map_err(|e| e.to_string())
        })
        .await
        .expect("script completes");
    let calls = calls.lock().unwrap().clone();
    (calls, serde_json::from_str(&out).unwrap())
}

#[tokio::test]
async fn a_task_that_declares_no_file_gets_one_noop_verification_and_no_write() {
    let unit = completion_unit("TASK-N");
    let (calls, result) = run("noop").await;
    let verify = format!("verification-wave-{unit}-verify-1");
    assert!(calls.contains(&verify), "{calls:?}");
    assert!(
        !calls.iter().any(|id| id.contains("-impl-")),
        "no write unit: {calls:?}"
    );
    assert_eq!(
        result["accepted"],
        serde_json::json!(["TASK-N"]),
        "{result}"
    );
    assert_eq!(
        result["task_completion"][0]["outcome"],
        serde_json::json!("accepted")
    );
}

#[tokio::test]
async fn a_failed_completion_the_script_never_remediated_is_remediated_before_acceptance() {
    // Accepted is not the no-op the host requires: the unit is blocked.
    let (calls, result) = run("accepted").await;
    assert_eq!(
        result["blocked"][0]["taskId"],
        serde_json::json!("TASK-N"),
        "{result}"
    );
    // The script ran no remediation pass; the prelude ran one over the
    // blocked task before the acceptance stage's first round.
    let plan = calls
        .iter()
        .position(|id| id.starts_with("remediation-plan-"))
        .unwrap_or_else(|| panic!("a review remediation pass ran: {calls:?}"));
    let round = calls
        .iter()
        .position(|id| id == "acceptance-contract-run-1")
        .unwrap_or_else(|| panic!("the acceptance stage ran: {calls:?}"));
    assert!(plan < round, "{calls:?}");
}
