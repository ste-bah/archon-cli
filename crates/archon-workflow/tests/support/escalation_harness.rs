//! The authored prelude driven end to end over the production write wave:
//! the real `remediateFindings` in QuickJS, every write through
//! `run_write_capable_v2_fanout` with real Git, every verdict scripted, and
//! each answer found the way the live host finds it -- an own-id record
//! whose input hash still matches, or `resume_drift::remediation_replay_record`
//! (drift and history), or the write path's own branch replay -- before
//! anything is dispatched. Views carry the host's escalation plan exactly as
//! the live `result_view` renders them.
#![allow(dead_code)]
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::call_data::{dispatched_items, fanout_items_for_call};
use archon_workflow::v2::script::remediation_escalation::{
    buys_escalation, escalation_refusal, refused_escalation_result, script_view,
};
use archon_workflow::v2::script::resume_drift::remediation_replay_record_escalating;
use archon_workflow::v2::script::resume_verdict::verdict_vouches_for_session_fix;
use archon_workflow::v2::script::{
    ScriptEnvelopeShape, completion_evidence_from_result, evidence_snapshot_hash,
    is_reusable_status, parse_script_options, result_view_json_shaped,
    reusable_record_has_required_completion_evidence, script_source,
};
use archon_workflow::v2::source_graph::input_hash_with_source_fingerprint;
use archon_workflow::*;
use serde_json::{Value, json};

use super::support::{AuditScript, Edits, Fixture};

/// The prelude this binary ships, and the one deployed at dfa009787.
pub const NEW_PRELUDE: &str = include_str!("../../src/v2/script/v3_primitives.js");
pub const OLD_PRELUDE: &str = include_str!("../fixtures/v3_primitives_dfa009787.js");
const OBJECTIVE: &str = "remediate review findings";

/// How the host answered one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Ran,
    Replayed,
    /// A no-patch checkpoint: no agent, recorded either way.
    Checkpoint,
    /// The host's dispatch check refused an escalated call.
    Refused(String),
}

/// One scripted verdict: accept, or refuse naming these blocker sources.
#[derive(Debug, Clone)]
pub enum Verdict {
    Accept,
    Refuse(Vec<&'static str>),
}

pub struct Host {
    pub f: Fixture,
    pub store: WorkflowV2ResultStore,
    /// Verdicts for calls the host dispatches, in order, per task key.
    pub verdicts: RefCell<Vec<(&'static str, VecDeque<Verdict>)>>,
    /// Edits a dispatched fix makes, by the unit key and whether escalated.
    pub edits: Box<dyn Fn(&str, u64, bool) -> Edits>,
    pub answers: RefCell<Vec<(String, Answer)>>,
    pub calls: RefCell<Vec<WorkflowV2HostCall>>,
    pub prompts: RefCell<Vec<(String, String)>>,
}

impl Host {
    pub fn new(
        f: Fixture,
        store: WorkflowV2ResultStore,
        edits: Box<dyn Fn(&str, u64, bool) -> Edits>,
    ) -> Self {
        Self {
            f,
            store,
            verdicts: RefCell::new(Vec::new()),
            edits,
            answers: RefCell::new(Vec::new()),
            calls: RefCell::new(Vec::new()),
            prompts: RefCell::new(Vec::new()),
        }
    }

    pub fn verdicts(&self, key: &'static str, list: Vec<Verdict>) {
        self.verdicts.borrow_mut().push((key, list.into()));
    }

    fn universe(&self) -> Option<&WorkflowV2TaskUniverse> {
        self.f.universe.as_ref()
    }

    fn execution(&self, method: &str, payload: &Value) -> WorkflowV2CallExecution {
        let method = WorkflowV2HostMethod::parse(method).expect("host method");
        let (options, write_mode) = parse_script_options(&payload["options"]).unwrap();
        let mut input = json!({
            "objective": OBJECTIVE, "call_id": payload["id"], "method": method.as_str(),
            "write_mode": write_mode, "options": payload["options"],
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

    /// The record as the live host writes it: dispatched items, completion
    /// evidence and its snapshot hash.
    fn save(
        &self,
        execution: &WorkflowV2CallExecution,
        result: WorkflowV2Result,
    ) -> WorkflowV2CallRecord {
        let evidence = completion_evidence_from_result(&result);
        // A new attempt over any earlier record, as the live host numbers it.
        let attempt = self
            .store
            .load_call_record(&execution.call.id)
            .unwrap()
            .map_or(1, |earlier| earlier.attempt + 1);
        let record = WorkflowV2CallRecord::new(
            self.f.run.clone(),
            execution.call.clone(),
            attempt,
            hash(execution),
            result,
            vec![],
        )
        .with_evidence_snapshot_hash(evidence_snapshot_hash(&evidence))
        .with_completion_evidence(evidence)
        .with_dispatched_items(dispatched_items(execution));
        self.store.save_call_record(&record).unwrap();
        record
    }

    /// The view through `script_view`, the function the live host's
    /// `result_view` calls, of the record that answered.
    fn view(&self, record: &WorkflowV2CallRecord) -> Value {
        let text = script_view(
            record,
            self.universe(),
            Some(&self.f.repo),
            ScriptEnvelopeShape::Deduped,
        )
        .unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// The live host lists a replayed call as the record that answered it
    /// (`mark_reused` pushes `record.call`), so the terminal rule reads the
    /// record the answer came from.
    fn note(&self, execution: &WorkflowV2CallExecution, answer: Answer) {
        self.note_as(execution, &execution.call, answer);
    }

    fn note_as(
        &self,
        execution: &WorkflowV2CallExecution,
        listed: &WorkflowV2HostCall,
        answer: Answer,
    ) {
        self.calls.borrow_mut().push(listed.clone());
        self.answers
            .borrow_mut()
            .push((execution.call.id.clone(), answer));
    }

    pub async fn answer(&self, method: &str, payload: Value) -> Value {
        if method == "checkpoint" {
            let execution = self.execution(method, &payload);
            if execution
                .call
                .options
                .extra
                .contains_key("remediationContract")
            {
                let _ = self.save(&execution, WorkflowV2Result::accepted("no patch"));
                self.note(&execution, Answer::Checkpoint);
            }
            return json!({"status": "accepted", "summary": "checkpoint"});
        }
        let execution = self.execution(method, &payload);
        // The live host's dispatch check, before any answer.
        if let Some(reason) =
            escalation_refusal(&execution, &self.store, self.universe(), Some(&self.f.repo))
        {
            self.note(&execution, Answer::Refused(reason.clone()));
            let text = result_view_json_shaped(
                &refused_escalation_result(&reason),
                ScriptEnvelopeShape::Deduped,
            )
            .unwrap();
            return serde_json::from_str(&text).unwrap();
        }
        if execution.call.write_mode.is_some() {
            return self.write(execution).await;
        }
        self.verify(execution)
    }

    async fn write(&self, execution: WorkflowV2CallExecution) -> Value {
        let contract = execution.call.options.extra["remediationContract"].clone();
        let key = contract["taskId"].as_str().unwrap().to_string();
        let escalated = contract.get("escalation").is_some();
        let round = contract["round"].as_u64().unwrap();
        let items = fanout_items_for_call(&execution, &self.store).unwrap();
        let task_ids: Vec<String> =
            serde_json::from_value(payload_item(&execution)["canonical_task_ids"].clone()).unwrap();
        let edits = (self.edits)(&key, round, escalated);
        let branches = items
            .into_iter()
            .map(|item| (item, edits.clone()))
            .collect();
        let (result, prompts) = self
            .f
            .wave_for(
                &self.store,
                execution.call.clone(),
                branches,
                (
                    Some(AuditScript {
                        flagged: vec![],
                        dispositions: std::collections::BTreeMap::new(),
                    }),
                    &[],
                    &[],
                ),
                task_ids,
                false,
            )
            .await;
        let answer = if prompts.is_empty() {
            Answer::Replayed
        } else {
            Answer::Ran
        };
        for prompt in prompts {
            self.prompts
                .borrow_mut()
                .push((execution.call.id.clone(), prompt));
        }
        let record = self.save(&execution, result);
        self.note(&execution, answer);
        self.view(&record)
    }

    fn verify(&self, execution: WorkflowV2CallExecution) -> Value {
        let records = self.store.load_call_records().unwrap();
        let vouched = |record: &WorkflowV2CallRecord| {
            verdict_vouches_for_session_fix(record, &records, &self.store)
        };
        let own = records.iter().find(|record| {
            record.call.id == execution.call.id
                && record.input_hash == hash(&execution)
                && is_reusable_status(record.status)
                && reusable_record_has_required_completion_evidence(record)
                && vouched(record)
        });
        let matches = |candidate: &WorkflowV2CallExecution, record: &WorkflowV2CallRecord| {
            record.input_hash == hash(candidate) && vouched(record)
        };
        let replay = own.or_else(|| {
            remediation_replay_record_escalating(
                &execution,
                &records,
                |id| self.store.in_session(id),
                matches,
                |record| buys_escalation(record, self.universe(), Some(&self.f.repo)),
            )
        });
        if let Some(record) = replay {
            self.store.note_session_call(&record.call.id);
            self.note_as(&execution, &record.call, Answer::Replayed);
            return self.view(record);
        }
        let key = execution.call.options.extra["remediationContract"]["taskId"]
            .as_str()
            .unwrap()
            .to_string();
        let verdict = {
            let mut verdicts = self.verdicts.borrow_mut();
            let queue = verdicts
                .iter_mut()
                .find(|(k, _)| *k == key)
                .map(|(_, queue)| queue);
            queue
                .and_then(VecDeque::pop_front)
                .unwrap_or(Verdict::Accept)
        };
        let result = verdict_result(&execution, &verdict);
        let record = self.save(&execution, result);
        self.note(&execution, Answer::Ran);
        self.view(&record)
    }
}

fn payload_item(execution: &WorkflowV2CallExecution) -> Value {
    execution.input["source_data"][0].clone()
}

pub fn hash(execution: &WorkflowV2CallExecution) -> String {
    input_hash_with_source_fingerprint(&execution.input, None)
}

/// A verifier branch's answer, in the shape the verification wave records.
fn verdict_result(execution: &WorkflowV2CallExecution, verdict: &Verdict) -> WorkflowV2Result {
    let item = payload_item(execution);
    let (status, summary, evidence) = match verdict {
        Verdict::Accept => ("accepted", "every finding resolved; baselines green", json!([
            {"kind": "test", "summary": "focused tests pass"}])),
        Verdict::Refuse(sources) => (
            "needs_review",
            "NOT accepted: must-pass baseline tests fail in another task's file",
            json!(sources.iter().map(|source| json!({"kind": "blocker",
                "summary": format!("{source} fails: the fix needs a change there"), "source": source}))
                .collect::<Vec<_>>()),
        ),
    };
    let branch = json!({"status": status, "summary": summary, "evidence": evidence,
        "commands_run": [{"kind": "test", "command": "cargo test", "status": "succeeded", "exit_code": 0}]});
    let tasks = item["canonical_task_ids"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let completion: Vec<Value> = tasks
        .iter()
        .map(|task| {
            json!({"task_id": task,
        "evidence_kind": "focused_verification", "call_id": execution.call.id,
        "item_id": item["item_id"], "status": status, "evidence_refs": ["scripted"]})
        })
        .collect();
    serde_json::from_value(json!({
        "status": status, "summary": summary, "evidence": evidence,
        "data": {"outcomes": [{"item_id": item["item_id"], "id": item["item_id"], "status": status,
            "canonical_task_ids": item["canonical_task_ids"], "result": branch,
            "completion_evidence": completion}],
            "items": [branch]},
    }))
    .unwrap()
}

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
    let out: String = context
        .async_with(async move |ctx| {
            ctx.globals()
                .set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let host = host.clone();
                        async move {
                            let payload: Value = serde_json::from_str(&payload).unwrap();
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

/// The repository file's content at HEAD.
pub fn at_head(repo: &Path, path: &str) -> String {
    super::support::git(repo, &["show", &format!("HEAD:{path}")])
}
