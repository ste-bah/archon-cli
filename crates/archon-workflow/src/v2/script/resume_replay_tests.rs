//! A replayed review call hands the script exactly what the fresh execution
//! handed it, so the reduce and remediation inputs built from it hash the
//! same. The host attaches and normalises findings ONCE, before the record is
//! persisted, and the fresh path returns the view of that persisted record;
//! a replay returns the same record's view and never re-normalises it.

use std::sync::{Arc, Mutex as StdMutex};

use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};

use super::*;
use crate::v2::result::{WorkflowV2Evidence, WorkflowV2EvidenceKind};
use crate::v2::script::{
    ScriptEnvelopeShape, normalize_and_attach_review_findings, result_view_json_shaped,
    script_source,
};
use crate::{
    WorkflowV2CallExecution, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2ResultStore,
};

fn map_execution() -> WorkflowV2CallExecution {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "reviewContract".to_string(),
        serde_json::json!({ "version": 1, "kind": "adversarial_findings", "stage": "map", "findingsPath": "data.findings", "itemTaskIdsPath": "canonical_task_ids" }),
    );
    WorkflowV2CallExecution {
        call: WorkflowV2HostCall {
            id: "adversarial-review-map".to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options,
        },
        input: serde_json::json!({
            "source_data": [{ "item_id": "review-task-a", "canonical_task_ids": ["TASK-A"], "task": "review" }],
        }),
        depends_on: Vec::new(),
    }
}

fn map_result() -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("fanout completed with findings");
    result.status = WorkflowV2Status::NeedsReview;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "reviewed",
    ));
    // Shapes the host rewrites: a spaced `task_id` spelling and a bare string.
    result.data = serde_json::json!({ "outcomes": [{
        "item_id": "adversarial-review-map-0", "status": "needs_review", "failure_kind": "semantic", "error": null,
        "result": { "status": "needs_review", "summary": "two findings", "data": { "findings": [
            { "id": "f1", "task_id": " TASK-A ", "claim": "the gate is unreachable" },
            "a bare finding the host wraps",
        ] } },
    }] });
    result
}

async fn reduce_payload(map_view: String) -> serde_json::Value {
    let calls: Arc<StdMutex<Vec<(String, serde_json::Value)>>> = Default::default();
    let source = script_source(
        "export const meta = { name: 'r', description: 'd', phases: [] }\nreturn await adversarialReview(['TASK-A'])\n",
        None,
    );
    let runtime = AsyncRuntime::new().expect("runtime");
    let context = AsyncContext::full(&runtime).await.expect("context");
    let recorded = calls.clone();
    context
        .async_with(async move |ctx| {
            ctx.globals()
                .set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let recorded = recorded.clone();
                        let map_view = map_view.clone();
                        async move {
                            let payload: serde_json::Value = serde_json::from_str(&payload).expect("payload");
                            let answer = if payload["id"] == "adversarial-review-map" {
                                map_view
                            } else {
                                serde_json::json!({ "status": "accepted", "summary": "stub", "review_findings": { "findings": [] } }).to_string()
                            };
                            recorded.lock().unwrap().push((method, payload));
                            Ok::<_, rquickjs::Error>(answer)
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
    calls
        .into_iter()
        .find(|(_, payload)| payload["id"] == "adversarial-review-reduce")
        .map(|(_, payload)| payload)
        .expect("the reduce was issued")
}

#[tokio::test]
async fn a_replayed_review_map_gives_the_reduce_the_input_the_fresh_one_gave_it() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let execution = map_execution();
    let attached = normalize_and_attach_review_findings(&execution, map_result(), &store, None)
        .expect("attach");
    let record = WorkflowV2CallRecord::new(
        "run",
        execution.call.clone(),
        1,
        "input".to_string(),
        attached,
        Vec::new(),
    );
    store.save_call_record(&record).expect("persist");
    // The fresh path returns the view of the record it persisted; a replay
    // loads that record back.
    let fresh =
        result_view_json_shaped(&record.result, ScriptEnvelopeShape::Deduped).expect("fresh view");
    let replayed_record = store
        .load_call_record("adversarial-review-map")
        .expect("load")
        .expect("stored");
    let replayed = result_view_json_shaped(&replayed_record.result, ScriptEnvelopeShape::Deduped)
        .expect("replay view");
    assert_eq!(
        fresh, replayed,
        "a replay hands the script the fresh view byte for byte"
    );
    assert!(
        completed_review_map_record(&replayed_record),
        "and the map is reusable"
    );
    let from_fresh = reduce_payload(fresh).await;
    let from_replay = reduce_payload(replayed).await;
    assert_eq!(
        from_fresh, from_replay,
        "so the reduce input -- and its hash -- is the same"
    );
    assert_eq!(
        from_replay["source"]["findings"], record.result.data["review_findings"]["findings"],
        "the prelude forwards the host's attachment verbatim"
    );
}

#[test]
fn a_replay_never_renormalises_what_an_older_host_attached() {
    // Records written before the host normalised attributions carry the raw
    // spelling. Downstream records were keyed on exactly that, so the replay
    // must hand it over unchanged; re-normalising would move every reduce and
    // remediation input hash on the first resume after a deploy.
    let mut result = map_result();
    result.data["review_findings"] = serde_json::json!({
        "source": "host", "kind": "adversarial_findings", "stage": "map",
        "findings": [{ "id": "f1", "task_id": " TASK-A ", "claim": "old shape" }],
    });
    let view: serde_json::Value = serde_json::from_str(
        &result_view_json_shaped(&result, ScriptEnvelopeShape::Deduped).expect("view"),
    )
    .expect("json");
    assert_eq!(view["review_findings"], result.data["review_findings"]);
}
