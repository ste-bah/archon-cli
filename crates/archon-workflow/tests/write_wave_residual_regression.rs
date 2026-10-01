//! Batch O2 (CUT-11) end to end: a regression one task's landing makes in
//! another task's declared test -- a NON-cargo command, compared at the run
//! base and the tip -- is found by the host's own regression check before
//! the second residual pass, routed to the task that declares the test as a
//! host-planned round through the real prelude, the production write wave
//! and the host's dispatch check, and fixed; the run goes green and the
//! post-script regression gate, the final judge, agrees. Without the check
//! the same run ends red at that gate, and a resume replays every call.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;
#[path = "support/residual_world.rs"]
mod world;

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use archon_workflow::agent_dispatch_port::{HostCommandEnv, WorkflowAgentDispatch};
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::script::{parse_script_options, script_source};
use archon_workflow::v2::verification::regression_gate::{RegressionGate, regression_verdict};
use archon_workflow::v2::verification::regression_slot::prepare_slot;
use archon_workflow::*;
use harness::{Answer, Host, NEW_PRELUDE, Verdict, at_head};
use serde_json::{Value, json};
use support::git;
use world::*;

/// TASK-B's declared test: B's lane must carry A's version.
const CHECK: &str = "sh check.sh";
const CHECK_SH: &str = "tok=$(sed -n 's/.*version=\\([0-9]*\\).*/\\1/p' crates/a/src/lib.rs)\ngrep -q \"version=$tok\" crates/b/src/lib.rs\n";

/// The host's runner environment: the plain shell, bounded.
struct Shell;

#[async_trait::async_trait]
impl WorkflowAgentDispatch for Shell {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn host_command_env(&self, _: &Path) -> HostCommandEnv {
        HostCommandEnv {
            vars: Vec::new(),
            hold: None,
        }
    }
    fn baseline_test_timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(20))
    }
    async fn run_call(
        &self,
        _: &str,
        _: Option<String>,
        _: &WorkflowV2CallExecution,
        _: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        _: Option<&WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        Err(WorkflowError::StageFailed("no agent runs here".into()))
    }
}

/// The residual world, both lanes at version 1, TASK-B declaring the check;
/// the review's cross-task round moves A to version 2, TASK-B's round moves
/// B there too.
fn regression_fixture() -> support::Fixture {
    let mut f = fixture();
    std::fs::write(f.repo.join(A), "// a version=1\n").unwrap();
    std::fs::write(f.repo.join(B), "// b version=1\n").unwrap();
    std::fs::write(f.repo.join("check.sh"), CHECK_SH).unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "versioned lanes and B's check"]);
    let base = git(&f.repo, &["rev-parse", "HEAD"]);
    let universe = f.universe.as_mut().unwrap();
    universe.tasks[1].focused_tests = vec![CHECK.into()];
    // The run is bound to the base, as the live host records it.
    let run = f.v2.root().parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&run).unwrap();
    use std::io::Write;
    let mut events = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(run.join("events.jsonl"))
        .unwrap();
    writeln!(
        events,
        "{}",
        json!({"seq": 1, "kind": "started", "detail": {
            "event": "repository_bound", "head": base, "drift": false}})
    )
    .unwrap();
    f
}

fn lanes(key: &str, _round: u64, _escalated: bool) -> support::Edits {
    match key {
        CROSS => edits(vec![(A, "// a: seam fixed version=2\n")]),
        "TASK-B" => edits(vec![(B, "// b version=2\n")]),
        _ => edits(vec![(STORE, "// store\n")]),
    }
}

fn session(f: support::Fixture) -> Rc<Host> {
    let store = WorkflowV2ResultStore::new(f.v2.root().to_path_buf());
    let host = Rc::new(Host::new(f, store, Box::new(lanes)));
    host.verdicts(CROSS, vec![Verdict::Accept]);
    host.verdicts("TASK-B", vec![Verdict::Accept]);
    host
}

/// `harness::run`, with the live host's step before it answers a residual
/// slot: the regression check (`prepare_slot`) when `check` is on.
async fn run(host: Rc<Host>, check: bool) -> Value {
    use rquickjs::function::{Async, Func};
    use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
    let source = script_source(&script(), None);
    let runtime = AsyncRuntime::new().unwrap();
    runtime.set_max_stack_size(8 * 1024 * 1024).await;
    let context = AsyncContext::full(&runtime).await.unwrap();
    let out: String = context
        .async_with(async move |ctx| {
            ctx.globals()
                .set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let host = host.clone();
                        async move {
                            let payload: Value = serde_json::from_str(&payload).unwrap();
                            if check && method == "checkpoint" {
                                let (options, write_mode) =
                                    parse_script_options(&payload["options"]).unwrap();
                                let call = WorkflowV2HostCall {
                                    id: payload["id"].as_str().unwrap().to_string(),
                                    method: WorkflowV2HostMethod::Checkpoint,
                                    write_mode,
                                    options,
                                };
                                prepare_slot(
                                    &host.store,
                                    &Shell,
                                    host.f.universe.as_ref(),
                                    &host.f.repo,
                                    &call,
                                )
                                .await
                                .expect("the slot's check is recorded");
                            }
                            let view = host.answer(&method, payload).await;
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

async fn final_gate(host: &Host) -> Vec<String> {
    regression_verdict(&RegressionGate {
        store: &host.store,
        dispatch: &Shell,
        universe: host.f.universe.as_ref(),
        repository_root: &host.f.repo,
    })
    .await
    .blocking
}

#[tokio::test]
async fn a_regression_found_before_the_second_pass_is_routed_to_its_owner_and_fixed() {
    assert!(NEW_PRELUDE.contains("residual-gaps-"));
    let host = session(regression_fixture());
    let result = run(host.clone(), true).await;
    let calls = residual_calls(&host);
    assert_eq!(
        calls.len(),
        2,
        "one round, fix and verifier: {:#?}",
        answers(&host)
    );
    let fix = host.store.load_call_record(&calls[0]).unwrap().unwrap();
    let contract = &fix.call.options.extra["remediationContract"];
    assert_eq!(contract["taskId"], json!("TASK-B"), "{contract}");
    assert_eq!(contract["residual"]["pass"], json!(2), "{contract}");
    let prompts = host.prompts.borrow();
    let (_, prompt) = prompts.iter().find(|(id, _)| *id == calls[0]).unwrap();
    assert!(
        prompt.contains("host_regression") && prompt.contains(CHECK),
        "{prompt}"
    );
    drop(prompts);
    assert_eq!(at_head(&host.f.repo, B), "// b version=2");
    let (status, why) = terminal(&host, &result);
    assert_eq!(status, WorkflowV2Status::Accepted, "{why}");
    // The final judge agrees: nothing regressed at the final tip.
    let blocking = final_gate(&host).await;
    assert!(blocking.is_empty(), "{blocking:#?}");

    // A resume replays every call: the slot's record is never rewritten, so
    // its pass plans exactly the round it planned.
    let Ok(first) = Rc::try_unwrap(host) else {
        panic!("the session is still referenced")
    };
    let second = session(first.f);
    run(second.clone(), true).await;
    let fresh: Vec<(String, Answer)> = answers(&second)
        .into_iter()
        .filter(|(_, answer)| *answer == Answer::Ran)
        .collect();
    assert!(fresh.is_empty(), "{fresh:#?}");
}

#[tokio::test]
async fn without_the_check_the_regression_reaches_the_final_gate_and_blocks() {
    let host = session(regression_fixture());
    run(host.clone(), false).await;
    assert!(residual_calls(&host).is_empty(), "{:#?}", answers(&host));
    assert_eq!(at_head(&host.f.repo, B), "// b version=1");
    let blocking = final_gate(&host).await;
    assert!(
        blocking.iter().any(|clause| clause.contains(CHECK)),
        "{blocking:#?}"
    );
}
