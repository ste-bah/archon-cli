//! Batch O: a live run's real review findings fed through this binary's
//! `remediateFindings`, with the host's remediation plan computed over the
//! run's task universe and the target repository. Ignored by default; run
//! with
//!   ARCHON_DRY_RUN_RUN=<run dir> ARCHON_DRY_RUN_REPO=<the target repository>
//!   [ARCHON_DRY_RUN_WATCH=<json: [{label, field, index, files}]>]
//!   [ARCHON_DRY_RUN_COPY=<a copy of the run dir, for landings and amendments>]
//!   cargo test -p archon-workflow --test dry_run_remediation_units -- --ignored --nocapture
//!
//! Nothing is written to the run: every fix is answered "nothing landed" (so
//! no verifier runs), and only the first-round fixes are inspected. It
//! proves every finding reaches a unit WHOLE (its prompt's findings parse
//! back equal to the finding plus its host id) and prints, for each watched
//! finding, its unit and whether each named file is among the unit's
//! writable targets.
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;

use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::review_finding_ids::finding_id_of;
use archon_workflow::v2::script::remediation_plan::with_remediation_plan;
use archon_workflow::v2::script::{
    ScriptEnvelopeShape, parse_script_options, result_view_json_shaped, script_source,
};
use archon_workflow::*;
use serde_json::{Value, json};

struct Stub {
    universe: WorkflowV2TaskUniverse,
    repo: PathBuf,
    /// A COPY of the run's store (`ARCHON_DRY_RUN_COPY`): the plan reads its
    /// landings and records its scope amendments there, never in the run.
    store: Option<WorkflowV2ResultStore>,
    /// (fix call id, contract, target files, prompt) of every fix.
    fixes: RefCell<Vec<(String, Value, Vec<String>, String)>>,
    /// The host's plan view, as the script read it.
    plan: RefCell<Value>,
}

impl Stub {
    fn answer(&self, method: &str, payload: Value) -> Value {
        let id = payload["id"].as_str().unwrap_or_default().to_string();
        let options = &payload["options"];
        if method == "checkpoint" && options.get("remediationPlan").is_some() {
            let (parsed, _) = parse_script_options(options).unwrap();
            let record = WorkflowV2CallRecord::new(
                "dry".to_string(),
                WorkflowV2HostCall {
                    id,
                    method: WorkflowV2HostMethod::Checkpoint,
                    write_mode: None,
                    options: parsed,
                },
                1,
                "dry".to_string(),
                WorkflowV2Result::accepted("plan"),
                vec![],
            );
            let viewed = with_remediation_plan(
                &record,
                &record.result,
                self.store.as_ref(),
                Some(&self.universe),
                Some(&self.repo),
            )
            .unwrap();
            let text = result_view_json_shaped(&viewed, ScriptEnvelopeShape::Deduped).unwrap();
            let view: Value = serde_json::from_str(&text).unwrap();
            *self.plan.borrow_mut() = view
                .get("remediation_plan")
                .cloned()
                .unwrap_or_else(|| view["data"]["remediation_plan"].clone());
            return view;
        }
        if method == "fanout" && options.get("write").is_some() {
            let item = &payload["source"][0];
            self.fixes.borrow_mut().push((
                id,
                options["remediationContract"].clone(),
                serde_json::from_value(item["target_files"].clone()).unwrap_or_default(),
                item["task"].as_str().unwrap_or_default().to_string(),
            ));
            return json!({"status": "failed", "summary": "dry run: not dispatched",
                "data": {"patch_landed": false}});
        }
        json!({"status": "accepted", "summary": "dry run"})
    }
}

/// The findings array a fix prompt carries, parsed back.
fn prompt_findings(prompt: &str) -> Vec<Value> {
    let start = prompt
        .find("Findings (verbatim):\n")
        .expect("findings section")
        + "Findings (verbatim):\n".len();
    let rest = &prompt[start..];
    let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
    stream.next().unwrap().unwrap().as_array().unwrap().clone()
}

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

#[tokio::test]
#[ignore = "needs a live run: ARCHON_DRY_RUN_RUN and ARCHON_DRY_RUN_REPO"]
async fn every_real_finding_reaches_a_unit_whole() {
    use rquickjs::function::{Async, Func};
    use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
    let (Some(run), Some(repo)) = (env("ARCHON_DRY_RUN_RUN"), env("ARCHON_DRY_RUN_REPO")) else {
        eprintln!("ARCHON_DRY_RUN_RUN / ARCHON_DRY_RUN_REPO unset; nothing to do");
        return;
    };
    let read =
        |path: PathBuf| -> Value { serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap() };
    let metadata = read(run.join("v2/generated-metadata.json"));
    let result = read(run.join("v2/script-result.json"));
    let findings: Vec<Value> = ["adversarial_findings", "uncovered_requirements"]
        .iter()
        .flat_map(|field| result[*field].as_array().cloned().unwrap_or_default())
        .collect();
    // The authored script's own task table, verbatim.
    let authored = std::fs::read_to_string(run.join("authored-workflow.js")).unwrap();
    let start = authored.find("const tasks = [").unwrap();
    let end = start + authored[start..].find("\n]\n").unwrap() + 3;
    let script = format!(
        "export const meta = {{ name: 'dry', description: 'd', phases: [] }}\n{}\nconst byId = (id) => tasks.find((t) => t.id === id) || {{}}\nconst review = await remediateFindings({}, {{ blockedTasks: {}, taskFileFor: (id) => byId(id).file, targetFilesFor: (id) => byId(id).targetFiles }})\nreturn {{ review }}\n",
        &authored[start..end],
        Value::from(findings.clone()),
        result["blocked"],
    );
    let stub = Rc::new(Stub {
        universe: serde_json::from_value(metadata["task_universe"].clone()).unwrap(),
        repo: repo.clone(),
        store: env("ARCHON_DRY_RUN_COPY").map(|copy| WorkflowV2ResultStore::new(copy.join("v2"))),
        fixes: RefCell::new(Vec::new()),
        plan: RefCell::new(Value::Null),
    });
    let source = script_source(&script, None);
    let runtime = AsyncRuntime::new().unwrap();
    runtime.set_max_stack_size(8 * 1024 * 1024).await;
    let context = AsyncContext::full(&runtime).await.unwrap();
    let answering = stub.clone();
    let out: String = context
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
        .await
        .expect("script completes");
    let review: Value = serde_json::from_str::<Value>(&out).unwrap()["review"].clone();

    // First-round fixes: one per unit.
    let fixes = stub.fixes.borrow();
    let first: Vec<&(String, Value, Vec<String>, String)> = fixes
        .iter()
        .filter(|(_, c, _, _)| c["round"] == json!(1) && c.get("escalation").is_none())
        .collect();
    let mut whole: BTreeMap<String, Vec<(String, Vec<String>)>> = BTreeMap::new();
    let mut largest = 0;
    for (_, contract, targets, prompt) in &first {
        largest = largest.max(prompt.len());
        let carried = prompt_findings(prompt);
        let named: BTreeSet<String> = contract["findingIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_string())
            .collect();
        assert_eq!(carried.len(), named.len(), "a unit carries exactly its ids");
        for finding in carried {
            let id = finding["finding_id"].as_str().unwrap().to_string();
            assert!(named.contains(&id));
            let unit = contract["unit"].as_str().unwrap().to_string();
            whole.entry(id).or_default().push((unit, targets.clone()));
        }
    }
    // Every finding, whole: its prompt copy minus the host id IS the finding.
    let mut missing = Vec::new();
    for finding in &findings {
        let id = finding_id_of(finding);
        let reached = first.iter().any(|(_, _, _, prompt)| {
            prompt_findings(prompt).iter().any(|carried| {
                let mut bare = carried.clone();
                bare.as_object_mut().unwrap().remove("finding_id");
                carried["finding_id"] == json!(id) && &bare == finding
            })
        });
        if !reached {
            missing.push(id);
        }
    }
    let units: BTreeSet<&str> = first
        .iter()
        .map(|(_, c, _, _)| c["unit"].as_str().unwrap())
        .collect();
    println!(
        "== findings {} reached whole {} missing {} | units {} | first-round fixes {} | largest prompt {} chars | unassigned {}",
        findings.len(),
        findings.len() - missing.len(),
        missing.len(),
        units.len(),
        first.len(),
        largest,
        review["unassigned"].as_array().map_or(0, Vec::len),
    );
    for unit in &units {
        let (_, contract, targets, _) = first
            .iter()
            .find(|(_, c, _, _)| c["unit"] == json!(unit))
            .unwrap();
        println!(
            "unit {unit}: {} finding(s), {} target(s){}",
            contract["findingIds"].as_array().unwrap().len(),
            targets.len(),
            contract
                .get("taskIds")
                .map(|t| format!(" spans {t}"))
                .unwrap_or_default()
        );
    }
    if let Some(watch) = env("ARCHON_DRY_RUN_WATCH") {
        let watched: Vec<Value> = serde_json::from_slice(&std::fs::read(watch).unwrap()).unwrap();
        let mut routed = 0;
        for entry in &watched {
            let finding = &result[entry["field"].as_str().unwrap()]
                [entry["index"].as_u64().unwrap() as usize];
            let id = finding_id_of(finding);
            let homes = whole.get(&id).cloned().unwrap_or_default();
            let files: Vec<String> = serde_json::from_value(entry["files"].clone()).unwrap();
            // Stored project data is granted on the plan (`project_grants`)
            // and stamped on the grantee's branches by the host.
            let offset = if entry["field"] == "uncovered_requirements" {
                result["adversarial_findings"]
                    .as_array()
                    .map_or(0, Vec::len)
            } else {
                0
            };
            let planned = stub.plan.borrow()["findings"]
                [offset + entry["index"].as_u64().unwrap() as usize]
                .clone();
            let granted: Vec<(String, bool)> = files
                .iter()
                .map(|file| {
                    let hit = match file.strip_prefix("project-data:") {
                        Some(path) => planned["project_grants"].get(path).is_some(),
                        None => homes.iter().any(|(_, targets)| targets.contains(file)),
                    };
                    (file.clone(), hit)
                })
                .collect();
            let ok = !homes.is_empty() && granted.iter().all(|(_, g)| *g);
            routed += usize::from(ok);
            println!(
                "watch {} ({id}): units {:?}; files {:?} => {}",
                entry["label"].as_str().unwrap(),
                homes.iter().map(|(u, _)| u.as_str()).collect::<Vec<_>>(),
                granted,
                if ok {
                    "ROUTED+WRITABLE"
                } else {
                    "NOT WRITABLE"
                }
            );
        }
        println!(
            "== watched {} routed with every named file writable {}",
            watched.len(),
            routed
        );
    }
    if let Some(copy) = env("ARCHON_DRY_RUN_COPY") {
        let ledger =
            archon_workflow::task_scope_amendment::ScopeAmendmentLedger::load(&copy).unwrap();
        println!(
            "== scope amendments recorded in the copy: {} grant(s), {} link(s)",
            ledger.set.grants.len(),
            ledger.lineage.len()
        );
        for grant in &ledger.set.grants {
            println!(
                "amendment {:?} {:?} {} -> {}",
                grant.kind, grant.root, grant.path, grant.task_id
            );
        }
    }
    assert!(
        missing.is_empty(),
        "findings that reached no unit whole: {missing:?}"
    );
    assert!(
        review["unassigned"].as_array().is_none_or(Vec::is_empty),
        "nothing is left unassigned"
    );
}
