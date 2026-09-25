//! Cross-task remediation: a finding that names tasks but that no single task
//! may act on is remediated by one write over the union of their files and
//! one verifier over all of them.

use crate::v2::script::script_source;
use std::sync::{Arc, Mutex as StdMutex};

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

const SCRIPT: &str = r#"export const meta = { name: 'cross', description: 'd', phases: [] }
const tasks = [
  { id: 'TASK-A', file: 'tasks/TASK-A.md', targetFiles: ['src/a.rs'] },
  { id: 'TASK-B', file: 'tasks/TASK-B.md', targetFiles: ['src/b.rs', 'src/shared.rs'] },
  { id: 'TASK-C', file: 'tasks/TASK-C.md', targetFiles: ['src/c.rs'] },
]
const byId = (id) => tasks.find((t) => t.id === id) || {}
const findings = [
  { id: 'own', canonical_task_ids: ['TASK-C'], severity: 'high' },
  { id: 'x1', attributable_to_task: false, canonical_task_ids: ['TASK-B', 'TASK-A'], severity: 'high' },
  { id: 'x2', attributable_to_task: false, task_ids: ['TASK-A', 'TASK-B'], severity: 'medium' },
  { id: 'prd', attributable_to_task: false, severity: 'low' },
]
const review_remediation = await remediateFindings(findings, { taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles })
return review_remediation
"#;

fn accepted_view() -> serde_json::Value {
    serde_json::json!({
        "status": "accepted", "summary": "stub", "items": [], "outcomes": [], "patch_landed": true,
        "result": { "status": "accepted", "summary": "stub", "files_changed": [{"path": "x"}], "commands_run": [{"command": "c", "status": "succeeded"}] },
    })
}

#[tokio::test]
async fn a_finding_no_single_task_owns_is_fixed_once_over_the_union_of_its_tasks() {
    let (calls, result) = run_scripted(SCRIPT, |_, _| accepted_view()).await;
    let cross: Vec<&(String, serde_json::Value)> = calls
        .iter()
        .filter(|(_, p)| p["options"]["remediationContract"]["taskId"] == "cross:TASK-A+TASK-B")
        .collect();
    assert_eq!(
        cross.len(),
        2,
        "one fix and one verifier for the group: {calls:#?}"
    );
    let (fix_method, fix) = cross[0];
    assert_eq!(fix_method, "fanout");
    assert_eq!(fix["options"]["remediationContract"]["stage"], "remediate");
    assert_eq!(
        fix["options"]["remediationContract"]["taskIds"],
        serde_json::json!(["TASK-A", "TASK-B"])
    );
    let item = &fix["source"][0];
    assert_eq!(
        item["canonical_task_ids"],
        serde_json::json!(["TASK-A", "TASK-B"])
    );
    assert_eq!(
        item["target_files"],
        serde_json::json!(["src/a.rs", "src/b.rs", "src/shared.rs"])
    );
    let prompt = item["task"].as_str().unwrap();
    assert!(
        prompt.contains("x1") && prompt.contains("x2") && !prompt.contains("\"own\""),
        "{prompt}"
    );
    let (verify_method, verify) = cross[1];
    assert_eq!(verify_method, "parallel");
    assert_eq!(verify["options"]["remediationContract"]["stage"], "verify");
    assert_eq!(
        verify["source"][0]["canonical_task_ids"],
        serde_json::json!(["TASK-A", "TASK-B"])
    );
    let result: serde_json::Value = serde_json::from_str(&result).unwrap();
    let resolved = result["resolved"].as_array().unwrap();
    assert!(resolved.iter().any(|e| e["taskId"] == "TASK-C"), "{result}");
    let group = resolved
        .iter()
        .find(|e| e["taskId"] == "cross:TASK-A+TASK-B")
        .expect("group resolved");
    assert_eq!(group["taskIds"], serde_json::json!(["TASK-A", "TASK-B"]));
    assert_eq!(group["crossTask"], true);
    assert_eq!(group["findingCount"], 2);
    let unassigned = result["unassigned"].as_array().unwrap();
    assert_eq!(
        unassigned.len(),
        1,
        "only the finding naming no task stays unassigned: {result}"
    );
    assert_eq!(unassigned[0]["id"], "prd");
}
