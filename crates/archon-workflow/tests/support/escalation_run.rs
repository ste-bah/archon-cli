//! Running a script through a prelude against the escalation harness host.
use std::path::Path;
use std::rc::Rc;

use archon_workflow::v2::script::script_source;
use serde_json::Value;

use super::{Host, NEW_PRELUDE};

/// Run `script` through `prelude` against `host`; the script's return value.
pub async fn run(script: &str, prelude: &str, host: Rc<Host>) -> Value {
    use rquickjs::function::{Async, Func};
    use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
    let source = script_source(script, None);
    assert!(
        source.contains(NEW_PRELUDE),
        "the prelude is embedded verbatim"
    );
    let source = source.replace(NEW_PRELUDE, prelude);
    let runtime = AsyncRuntime::new().unwrap();
    runtime.set_max_stack_size(8 * 1024 * 1024).await;
    let context = AsyncContext::full(&runtime).await.unwrap();
    let observed = host.clone();
    let out = context
        .async_with(async move |ctx| {
            ctx.globals()
                .set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let host = host.clone();
                        async move {
                            let payload: Value = serde_json::from_str(&payload).unwrap();
                            let generation = host.f.store.load_state(&host.f.run).unwrap().generation;
                            let view = host.answer(&method, payload.clone()).await;
                            if let Some(evidence) = payload["options"].get("remediationPause")
                                .or_else(|| view.get("remediation_pause")) {
                                archon_workflow::control_pause::pause_with_evidence(
                                    &host.f.store, &host.f.run, generation,
                                    serde_json::json!({"event":"remediation_stall_pause","evidence":evidence}),
                                ).unwrap().unwrap();
                                // As the live host: a residual stop is waived for the resume.
                                if let Some(pass) = evidence.get("pass").and_then(Value::as_u64) {
                                    host.store.waive_residual_stall(pass).unwrap();
                                }
                                return Err(rquickjs::Error::Unknown);
                            }
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
        .await;
    match out {
        Ok(out) => serde_json::from_str::<Value>(&out).unwrap(),
        Err(_)
            if observed.f.store.load_state(&observed.f.run).unwrap().status
                == archon_workflow::RunStatus::Paused =>
        {
            serde_json::json!({"paused":true})
        }
        Err(error) => panic!("script failed without pausing: {error}"),
    }
}

/// Remediation that stopped making progress paused the run (Issue 262):
/// the script returned `{"paused": true}`, the run is `Paused`, and its
/// pause evidence names `task` -- never a terminal `NeedsReview`.
pub fn assert_stall_paused(host: &Host, result: &Value, task: &str) {
    assert_eq!(result, &serde_json::json!({"paused": true}), "{result}");
    assert_eq!(
        host.f.store.load_state(&host.f.run).unwrap().status,
        archon_workflow::RunStatus::Paused
    );
    let events = std::fs::read_to_string(host.f.store.events_path(&host.f.run)).unwrap();
    assert!(
        events.contains("remediation_stall_pause") && events.contains(task),
        "{events}"
    );
}

/// The repository file's content at HEAD.
pub fn at_head(repo: &Path, path: &str) -> String {
    super::super::support::git(repo, &["show", &format!("HEAD:{path}")])
}
