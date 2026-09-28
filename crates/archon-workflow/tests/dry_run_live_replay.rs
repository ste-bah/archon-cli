//! A replay dry run of a live run's authored script against a COPY of its
//! records. Ignored by default; run with
//!   ARCHON_DRY_RUN_RUN=<copied run dir> ARCHON_DRY_RUN_REPO=<a clone of the
//!   target at the run's HEAD> [ARCHON_DRY_RUN_PRELUDE=<prelude .js>]
//!   cargo test -p archon-workflow --test dry_run_live_replay -- --ignored --nocapture
//!
//! The script runs through the prelude (this binary's, or the named one) and
//! every host call is answered the way the live host would find it: a
//! record under the call's id whose input hash -- computed from the call's
//! input exactly as the live host builds it, with the record's own source
//! fingerprint -- still matches is IDENTICAL and answered with the host's
//! view of it; a record that matches but is not reusable is RERUN (the live
//! host replays its verdict from history, or dispatches it again under the
//! same id -- the script reads the recorded verdict); a record whose hash
//! differs is DIFFERS -- a replay break; no record at all is NEW. New
//! checkpoints and re-run acceptance rounds are recorded in the copy (as
//! the live host records them), and a write fan-out's branch decision saves
//! what the live split saves (migrated, landing and refiled outcomes). It
//! prints every call and the totals, and fails on any DIFFERS. An
//! acceptance round (never replayed by the host) is RERUN.
//!
//! Batch H: a dynamic write fan-out is judged per branch and a remediation
//! verdict by its fix's lineage, as live (`verdict`); a NEW call answers the
//! script with a not-dispatched stub, so a full-mode replay leaves the live
//! path at its first NEW call -- the ordinal mode is the faithful one there.
//!
//! With `ARCHON_DRY_RUN_ORDINAL=<n>` it replays only the final stage: the
//! script's declarations (every line before its first top-level loop or
//! `await`), `n`
//! ordinal-advancing `log()` markers -- the prelude's call ordinal as the
//! live session reached `acceptance()` -- and the script's own
//! `acceptance(...)` call. What precedes that stage is prelude code this
//! batch does not change; the final stage is where host-planned units are
//! skipped, finished and filed. `log-*` markers are never compared: they
//! are UI markers, not cached work.
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::script::remediation_escalation::script_view_in;
use archon_workflow::v2::script::{
    ScriptEnvelopeShape, is_reusable_status, parse_script_options, script_source,
};
use archon_workflow::v2::source_graph::input_hash_with_source_fingerprint;
use archon_workflow::*;
use serde_json::{Value, json};

const PRELUDE: &str = include_str!("../src/v2/script/v3_primitives.js");

struct Replay {
    store: WorkflowV2ResultStore,
    universe: WorkflowV2TaskUniverse,
    repo: PathBuf,
    objective: String,
    run: String,
    seen: RefCell<Vec<(String, &'static str)>>,
}

impl Replay {
    fn execution(&self, method: &str, payload: &Value) -> WorkflowV2CallExecution {
        let method = WorkflowV2HostMethod::parse(method).expect("host method");
        let (options, write_mode) = parse_script_options(&payload["options"]).unwrap();
        let mut input = json!({
            "objective": self.objective, "call_id": payload["id"], "method": method.as_str(),
            "write_mode": write_mode, "options": payload["options"],
        });
        if let Some(source) = payload.get("source") {
            input["source_data"] = source.clone();
        }
        if let Some(inputs) = payload["options"].get("inputs").cloned() {
            input["inputs"] = inputs.clone();
            if payload.get("source").is_none() {
                input["source_data"] = inputs;
            }
        }
        if let Some(source) = options.source.as_deref() {
            input["source"] = Value::String(source.to_string());
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
        let text = script_view_in(
            record,
            &self.store,
            Some(&self.universe),
            Some(&self.repo),
            ScriptEnvelopeShape::Deduped,
        )
        .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// The live host's decision for this call, and the record that answers
    /// it when no record under its own id does (a drifted sibling).
    ///
    /// Batch H: the same decision as live, not a hash comparison alone. A
    /// dynamic write fan-out is never replayed whole live: its branches
    /// decide (`split_reusable_branch_outcomes`, which also notes the fix
    /// lineage a verdict needs), so it is IDENTICAL only when every branch
    /// is reused. A recorded remediation verdict stands only while this
    /// session's fix was replayed from the fix it judged, and no remediation
    /// answer stands for a question observed after it
    /// (`resume_freshness`). Live, 012-1-48 and 013-1-50 re-ran on the
    /// verdict rule while this tool, comparing hashes only, called them
    /// IDENTICAL.
    fn verdict(
        &self,
        execution: &WorkflowV2CallExecution,
        record: Option<&WorkflowV2CallRecord>,
    ) -> (&'static str, Option<WorkflowV2CallRecord>) {
        use archon_workflow::v2::script::resume_drift::{
            is_remediation_call, remediation_replay_record_escalating,
        };
        use archon_workflow::v2::script::resume_freshness::record_predates_question;
        use archon_workflow::v2::script::resume_verdict::verdict_vouches_for_session_fix;
        // An acceptance round is never replayed by the host: it re-runs
        // against the tree as it is, whatever it was asked before.
        if archon_workflow::v2::script::is_acceptance_stage_call(&execution.call) {
            return (if record.is_some() { "RERUN" } else { "NEW" }, None);
        }
        let call = &execution.call;
        if call.write_mode.is_some() && call.options.target_files_from_item {
            return self.branches_verdict(execution, record.is_some());
        }
        let records = self.store.load_call_records().unwrap();
        let hash_of = |execution: &WorkflowV2CallExecution, record: &WorkflowV2CallRecord| {
            input_hash_with_source_fingerprint(
                &execution.input,
                record.source_fingerprint.as_deref(),
            )
        };
        let stands = |record: &WorkflowV2CallRecord| {
            verdict_vouches_for_session_fix(record, &records, &self.store)
                && !record_predates_question(&self.store, call, record, &records)
        };
        if let Some(record) = record {
            return match (
                hash_of(execution, record) == record.input_hash,
                is_reusable_status(record.status) && stands(record),
            ) {
                (true, true) => ("IDENTICAL", None),
                (true, false) => ("RERUN", None),
                (false, _) => ("DIFFERS", None),
            };
        }
        if !is_remediation_call(call) {
            return ("NEW", None);
        }
        let matches = |candidate: &WorkflowV2CallExecution, record: &WorkflowV2CallRecord| {
            hash_of(candidate, record) == record.input_hash && stands(record)
        };
        let in_session = |id: &str| self.store.in_session(id);
        let escalates = |record: &WorkflowV2CallRecord| {
            archon_workflow::v2::script::remediation_escalation::buys_escalation(
                record,
                Some(&self.universe),
                Some(&self.repo),
            )
        };
        match remediation_replay_record_escalating(
            execution, &records, in_session, matches, escalates,
        ) {
            Some(sibling) => ("IDENTICAL", Some(sibling.clone())),
            None => ("NEW", None),
        }
    }

    /// A dynamic write fan-out as the live write path decides it: the
    /// host's item builder, the authored identity and drift identities
    /// stamped first (as `run_write_capable_v2_fanout` does), the repository
    /// root the tree checks read, then the branch split.
    fn branches_verdict(
        &self,
        execution: &WorkflowV2CallExecution,
        recorded: bool,
    ) -> (&'static str, Option<WorkflowV2CallRecord>) {
        let mut items =
            archon_workflow::v2::call_data::fanout_items_for_call(execution, &self.store).unwrap();
        archon_workflow::v2::reuse_identity::stamp_reuse_input_hash(&mut items);
        archon_workflow::v2::branch_cache::stamp_drift_identities(
            &mut items,
            &execution.call.id,
            &self.store,
        )
        .unwrap();
        for item in &mut items {
            if let Some(object) = item.input.get_mut("item").and_then(Value::as_object_mut) {
                object
                    .entry("target_repository_root")
                    .or_insert_with(|| Value::String(self.repo.display().to_string()));
            }
        }
        let (reused, pending) = archon_workflow::v2::branch_cache::split_reusable_branch_outcomes(
            &self.store,
            &execution.call.id,
            items,
        )
        .unwrap();
        // Answered by a drifted sibling with no record of its own: the
        // script reads the record the fix was replayed from.
        let source =
            archon_workflow::v2::script::resume_verdict::remediation_round_key(&execution.call)
                .and_then(|key| self.store.fix_replayed_from(&key))
                .and_then(|id| self.store.load_call_record(&id).unwrap());
        match (pending.is_empty() && !reused.is_empty(), recorded) {
            (true, _) => ("IDENTICAL", source),
            (false, true) => ("RERUN", None),
            (false, false) => ("NEW", None),
        }
    }

    fn answer(&self, method: &str, payload: Value) -> Value {
        let execution = self.execution(method, &payload);
        let id = execution.call.id.clone();
        if execution.call.method == WorkflowV2HostMethod::Checkpoint && id.starts_with("log-") {
            return json!({"status": "accepted", "summary": "marker"});
        }
        let mut record = self.store.load_call_record(&id).unwrap();
        let (verdict, answer) = self.verdict(&execution, record.as_ref());
        // An acceptance round re-runs live and is recorded as it runs: the
        // observation every remediation after it is judged against
        // (`resume_freshness`). Its recorded answer stands in for the result.
        if archon_workflow::v2::script::is_acceptance_stage_call(&execution.call)
            && let Some(previous) = record.take()
        {
            let rerun = WorkflowV2CallRecord::new(
                self.run.clone(),
                previous.call.clone(),
                previous.attempt + 1,
                previous.input_hash.clone(),
                previous.result.clone(),
                vec![],
            );
            self.store.save_call_record(&rerun).unwrap();
            record = Some(rerun);
        }
        println!("{verdict:9} {id}");
        self.seen.borrow_mut().push((id.clone(), verdict));
        // A recorded answer is what the live host's history replay hands
        // back for a call it does not dispatch again, and what it records
        // when it does; either way the script reads the recorded verdict.
        if let Some(record) = record.filter(|_| verdict != "DIFFERS").or(answer) {
            self.store.note_session_call(&id);
            return self.view(&record);
        }
        if execution.call.method == WorkflowV2HostMethod::Checkpoint {
            let record = WorkflowV2CallRecord::new(
                self.run.clone(),
                execution.call.clone(),
                1,
                input_hash_with_source_fingerprint(&execution.input, None),
                WorkflowV2Result::accepted("dry-run checkpoint"),
                vec![],
            );
            self.store.save_call_record(&record).unwrap();
            self.store.note_session_call(&id);
            return self.view(&record);
        }
        // Not dispatched in a dry run: the script is told nothing ran.
        json!({"status": "needs_review", "summary": "dry run: not dispatched", "final": true})
    }
}

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

async fn replay(run: &Path, repo: &Path, prelude: &str) -> Vec<(String, &'static str)> {
    use rquickjs::function::{Async, Func};
    use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(run.join("v2/generated-metadata.json")).unwrap())
            .unwrap();
    let state: Value =
        serde_json::from_slice(&std::fs::read(run.join("state.json")).unwrap()).unwrap();
    let host = Rc::new(Replay {
        store: WorkflowV2ResultStore::new(run.join("v2")),
        universe: serde_json::from_value(metadata["task_universe"].clone()).unwrap(),
        repo: repo.to_path_buf(),
        objective: state["spec"]["task"].as_str().unwrap().to_string(),
        run: state["id"].as_str().unwrap().to_string(),
        seen: RefCell::new(Vec::new()),
    });
    let mut authored = std::fs::read_to_string(run.join("authored-workflow.js")).unwrap();
    if let Ok(ordinal) = std::env::var("ARCHON_DRY_RUN_ORDINAL") {
        let ordinal: usize = ordinal.parse().unwrap();
        let lines: Vec<&str> = authored.lines().collect();
        // The first top-level statement that does work: a loop, or an await.
        let first_await = lines
            .iter()
            .position(|l| {
                l.starts_with("for ")
                    || l.starts_with("while ")
                    || (l.contains("await ") && !l.starts_with([' ', '\t']))
            })
            .unwrap();
        let acceptance = lines
            .iter()
            .find(|l| l.contains("await acceptance("))
            .expect("the script's acceptance call");
        authored = format!(
            "{}\nfor (let i = 0; i < {ordinal}; i += 1) log('dry-run ordinal')\n{acceptance}\nreturn {{}}\n",
            lines[..first_await].join("\n")
        );
    }
    let source = script_source(&authored, None).replace(PRELUDE, prelude);
    let runtime = AsyncRuntime::new().unwrap();
    runtime.set_max_stack_size(8 * 1024 * 1024).await;
    let context = AsyncContext::full(&runtime).await.unwrap();
    let answering = host.clone();
    let out: Result<String, String> = context
        .async_with(async move |ctx| {
            ctx.globals()
                .set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let host = answering.clone();
                        async move {
                            let payload: Value = serde_json::from_str(&payload).unwrap();
                            Ok::<_, rquickjs::Error>(host.answer(&method, payload).to_string())
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
    println!(
        "== script ended: {}",
        out.map(|_| "returned".to_string()).unwrap_or_else(|e| e)
    );
    host.seen.borrow().clone()
}

#[tokio::test]
#[ignore = "needs a copied live run: ARCHON_DRY_RUN_RUN and ARCHON_DRY_RUN_REPO"]
async fn dry_run_replays_the_live_run() {
    let (Some(run), Some(repo)) = (env("ARCHON_DRY_RUN_RUN"), env("ARCHON_DRY_RUN_REPO")) else {
        eprintln!("ARCHON_DRY_RUN_RUN / ARCHON_DRY_RUN_REPO unset; nothing to do");
        return;
    };
    let prelude = env("ARCHON_DRY_RUN_PRELUDE")
        .map(|path| std::fs::read_to_string(path).unwrap())
        .unwrap_or_else(|| PRELUDE.to_string());
    let seen = replay(&run, &repo, &prelude).await;
    let count = |which: &str| seen.iter().filter(|(_, v)| *v == which).count();
    println!(
        "== IDENTICAL {} RERUN {} NEW {} DIFFERS {}",
        count("IDENTICAL"),
        count("RERUN"),
        count("NEW"),
        count("DIFFERS")
    );
    assert_eq!(count("DIFFERS"), 0, "a recorded call would not replay");
}
