//! A review-answering front for the escalation harness (REM-10): review map
//! and reduce calls are answered here with scripted per-task findings, put
//! through the live host's normalization and attachment
//! (`normalize_and_attach_review_findings`) and
//! recorded as the live host records them; a recorded review call whose
//! input is unchanged replays. Every other call goes to the harness.
#![allow(dead_code)]
use std::rc::Rc;

use archon_workflow::v2::call_data::dispatched_items;
use archon_workflow::v2::script::remediation_escalation::script_view_in;
use archon_workflow::v2::script::{
    ScriptEnvelopeShape, normalize_and_attach_review_findings, parse_script_options, script_source,
};
use archon_workflow::*;
use serde_json::{Value, json};

use super::harness::{Answer, Host, NEW_PRELUDE, hash};

/// What a map branch returns: (map call id, task id, findings).
pub type MapScript = Vec<(&'static str, &'static str, Vec<Value>)>;

pub struct Reviewer {
    pub host: Rc<Host>,
    pub maps: MapScript,
    /// Called with each map's attached findings once it is answered live.
    pub on_map: Box<dyn Fn(&Host, &str, &[Value])>,
}

impl Reviewer {
    fn execution(method: &str, payload: &Value) -> WorkflowV2CallExecution {
        let method = WorkflowV2HostMethod::parse(method).expect("host method");
        let (options, write_mode) = parse_script_options(&payload["options"]).unwrap();
        let mut input = json!({
            "objective": "remediate review findings", "call_id": payload["id"],
            "method": method.as_str(), "write_mode": write_mode, "options": payload["options"],
        });
        if let Some(source) = payload.get("source") {
            input["source_data"] = source.clone();
        }
        WorkflowV2CallExecution {
            call: WorkflowV2HostCall {
                id: payload["id"].as_str().unwrap().to_string(),
                method,
                write_mode,
                options,
            },
            input,
            depends_on: vec![],
        }
    }

    fn view(&self, record: &WorkflowV2CallRecord) -> Value {
        let host = &self.host;
        let text = script_view_in(
            record,
            &host.store,
            host.f.universe.as_ref(),
            Some(&host.f.repo),
            ScriptEnvelopeShape::Deduped,
        )
        .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn note(&self, call: &WorkflowV2HostCall, answer: Answer) {
        self.host.calls.borrow_mut().push(call.clone());
        self.host
            .answers
            .borrow_mut()
            .push((call.id.clone(), answer));
    }

    fn map_result(&self, execution: &WorkflowV2CallExecution) -> WorkflowV2Result {
        let items = dispatched_items(execution);
        let mut any = false;
        let outcomes: Vec<Value> = items
            .iter()
            .map(|item| {
                let task = item.canonical_task_ids[0].clone();
                let findings: Vec<Value> = self
                    .maps
                    .iter()
                    .filter(|(call, of, _)| *call == execution.call.id && *of == task)
                    .flat_map(|(_, _, findings)| findings.clone())
                    .collect();
                any |= !findings.is_empty();
                let status = if findings.is_empty() {
                    "accepted"
                } else {
                    "needs_review"
                };
                json!({"item_id": item.item_id, "id": item.item_id, "role": "critic", "status": status,
                    "canonical_task_ids": [task],
                    "result": {"status": status, "summary": "reviewed",
                        "data": {"findings": findings, "canonical_task_ids": [task]}}})
            })
            .collect();
        WorkflowV2Result {
            data: json!({ "outcomes": outcomes }),
            ..if any {
                WorkflowV2Result {
                    status: WorkflowV2Status::NeedsReview,
                    ..WorkflowV2Result::accepted("findings")
                }
            } else {
                WorkflowV2Result::accepted("reviewed clean")
            }
        }
    }

    pub async fn answer(&self, method: &str, payload: Value) -> Value {
        if payload["options"].get("reviewContract").is_none() {
            return self.host.answer(method, payload).await;
        }
        let execution = Self::execution(method, &payload);
        let store = &self.host.store;
        if let Some(record) = store
            .load_call_record(&execution.call.id)
            .unwrap()
            .filter(|record| record.input_hash == hash(&execution))
        {
            store.note_session_call(&record.call.id);
            self.note(&record.call, Answer::Replayed);
            return self.view(&record);
        }
        let result = if execution.call.method == WorkflowV2HostMethod::Reduce {
            WorkflowV2Result {
                data: json!({ "findings": [] }),
                ..WorkflowV2Result::accepted("no cross-task finding")
            }
        } else {
            self.map_result(&execution)
        };
        let result = normalize_and_attach_review_findings(
            &execution,
            result,
            store,
            self.host.f.universe.as_ref(),
        )
        .unwrap();
        let record = WorkflowV2CallRecord::new(
            self.host.f.run.clone(),
            execution.call.clone(),
            1,
            hash(&execution),
            result,
            vec![],
        )
        .with_dispatched_items(dispatched_items(&execution));
        store.save_call_record(&record).unwrap();
        self.note(&execution.call, Answer::Ran);
        if execution.call.method != WorkflowV2HostMethod::Reduce {
            let attached = record.result.data["review_findings"]["findings"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            (self.on_map)(&self.host, &execution.call.id, &attached);
        }
        self.view(&record)
    }
}

/// Run `script` through the shipped prelude against `reviewer`.
pub async fn run_reviewing(script: &str, reviewer: Rc<Reviewer>) -> Value {
    use rquickjs::function::{Async, Func};
    use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
    let source = script_source(script, None);
    assert!(
        source.contains(NEW_PRELUDE),
        "the prelude is embedded verbatim"
    );
    let runtime = AsyncRuntime::new().unwrap();
    runtime.set_max_stack_size(8 * 1024 * 1024).await;
    let context = AsyncContext::full(&runtime).await.unwrap();
    let out: String = context
        .async_with(async move |ctx| {
            ctx.globals()
                .set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let reviewer = reviewer.clone();
                        async move {
                            let payload: Value = serde_json::from_str(&payload).unwrap();
                            let view = reviewer.answer(&method, payload).await;
                            Ok::<_, rquickjs::Error>(view.to_string())
                        }
                    })),
                )
                .unwrap();
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
    serde_json::from_str(&out).unwrap()
}

/// Append verdicts to `key`'s queue (the harness answers from the first
/// queue a key has).
pub fn queue_verdicts(host: &Host, key: &'static str, list: Vec<super::harness::Verdict>) {
    let mut verdicts = host.verdicts.borrow_mut();
    match verdicts.iter_mut().find(|(k, _)| *k == key) {
        Some((_, queue)) => queue.extend(list),
        None => verdicts.push((key, list.into())),
    }
}
