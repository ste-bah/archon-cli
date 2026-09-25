//! Remediation call labels keep their round and unit. `agent()` cuts a label
//! to 40 characters before appending the ordinal, and a long unit key (every
//! cross-task unit) used to lose its round there: two rounds, or two units,
//! shared one label. Labels that already fit are unchanged, so records filed
//! under them still replay.

use crate::v2::script::history_replay::call_family;
use crate::v2::script::script_source;
use std::sync::{Arc, Mutex as StdMutex};

async fn call_ids(source: &str) -> Vec<(String, serde_json::Value)> {
    use rquickjs::function::{Async, Func};
    use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
    let calls: Arc<StdMutex<Vec<(String, serde_json::Value)>>> = Default::default();
    let source = script_source(source, None);
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
                        async move {
                            let payload: serde_json::Value =
                                serde_json::from_str(&payload).expect("payload");
                            let mut calls = recorded.lock().unwrap();
                            let round = payload["options"]["remediationContract"]["round"].clone();
                            // Round 1's verifier rejects, round 2's accepts.
                            let status = if method == "parallel" && round == 1 {
                                "needs_review"
                            } else {
                                "accepted"
                            };
                            calls.push((payload["id"].as_str().unwrap_or("").to_string(), payload));
                            let view = serde_json::json!({
                                "status": status, "summary": "stub", "items": [], "outcomes": [], "patch_landed": true,
                                "result": { "status": status, "summary": "stub", "files_changed": [{"path": "x"}],
                                    "commands_run": [{"command": "c", "status": "succeeded"}] },
                            });
                            Ok::<_, rquickjs::Error>(view.to_string())
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
        .map(|(id, payload)| (id, payload["options"]["remediationContract"].clone()))
        .collect()
}

const SCRIPT: &str = r#"export const meta = { name: 'labels', description: 'd', phases: [] }
const files = { 'TASK-TRADING-002': ['src/b.rs'], 'TASK-DATA-LAKE-INGEST-ALPHA': ['src/alpha.rs'], 'TASK-DATA-LAKE-INGEST-BETA': ['src/beta.rs'] }
const findings = [
  { id: 'own', canonical_task_ids: ['TASK-TRADING-002'] },
  { id: 'x1', attributable_to_task: false, canonical_task_ids: ['TASK-DATA-LAKE-INGEST-ALPHA', 'TASK-DATA-LAKE-INGEST-BETA'] },
  { id: 'x2', attributable_to_task: false, canonical_task_ids: ['TASK-DATA-LAKE-INGEST-ALPHA', 'TASK-TRADING-002'] },
]
return await remediateFindings(findings, { taskFileFor: (id) => 'tasks/' + id + '.md', targetFilesFor: (id) => files[id] })
"#;

/// The label a call id was minted under, the verifier wave prefix dropped.
fn label(id: &str) -> &str {
    let (label, _) = call_family(id);
    label.strip_prefix("verification-wave-").unwrap_or(label)
}

#[tokio::test]
async fn every_round_and_unit_gets_its_own_label() {
    let calls = call_ids(SCRIPT).await;
    let mut seen = std::collections::BTreeMap::<String, serde_json::Value>::new();
    for (id, contract) in &calls {
        if contract.is_null() {
            continue;
        }
        let label = label(id).to_string();
        assert!(label.len() <= 40, "{label}");
        let key = serde_json::json!([contract["stage"], contract["taskId"], contract["round"]]);
        if let Some(previous) = seen.insert(label.clone(), key.clone()) {
            assert_eq!(
                previous, key,
                "two remediation questions share label {label}"
            );
        }
        let round = contract["round"].as_u64().expect("round");
        assert!(
            label.ends_with(&format!("-{round}")),
            "{label} lost its round {round}"
        );
    }
    let crosses = calls
        .iter()
        .filter(|(_, contract)| {
            contract["taskId"]
                .as_str()
                .is_some_and(|t| t.starts_with("cross:"))
        })
        .count();
    assert_eq!(
        crosses, 8,
        "two cross units, two rounds, fix and verify: {calls:#?}"
    );
}

#[tokio::test]
async fn a_label_that_fits_is_unchanged() {
    let calls = call_ids(SCRIPT).await;
    let labels: Vec<&str> = calls
        .iter()
        .filter(|(_, contract)| contract["taskId"] == "TASK-TRADING-002")
        .map(|(id, _)| label(id))
        .collect();
    assert_eq!(
        labels,
        vec![
            "review-remediate-task-trading-002-1",
            "review-verify-task-trading-002-1",
            "review-remediate-task-trading-002-2",
            "review-verify-task-trading-002-2",
        ]
    );
}
