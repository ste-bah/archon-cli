//! Issue-117: what the host plans for residual gaps, what it lets a round
//! dispatch, and what the final gate concludes.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::result_store::WorkflowV2DispatchedItem;
use crate::v2::{
    WorkflowV2CallExecution, WorkflowV2HostCall, WorkflowV2HostOptions, WorkflowV2ResidualGap,
    WorkflowV2Status, WorkflowV2WriteMode,
};
use crate::v2::{WorkflowV2Result, WorkflowV2ResultStore};
use serde_json::json;

pub(super) const STORE: &str = "crates/shared/src/store.rs";

pub(super) struct World {
    pub(super) dir: tempfile::TempDir,
    pub(super) store: WorkflowV2ResultStore,
    pub(super) universe: WorkflowV2TaskUniverse,
}

impl World {
    pub(super) fn root(&self) -> &Path {
        self.dir.path()
    }

    pub(super) fn plan(&self) -> ResidualPlan {
        let records = session_records(&self.store);
        let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
        plan_from(&refs, Some(&self.universe), Some(self.root()))
    }

    pub(super) fn save(&self, record: &WorkflowV2CallRecord) {
        self.store.save_call_record(record).unwrap();
        self.store.note_session_call(&record.call.id);
    }
}

pub(super) fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "crates/a/src/lib.rs",
        "crates/b/src/lib.rs",
        STORE,
        "tasks/TASK-A.md",
        "tasks/TASK-B.md",
    ] {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    std::fs::write(
        dir.path().join("tasks/TASK-A.md"),
        format!("`{STORE}` and the ingest lane must stay consistent\n"),
    )
    .unwrap();
    let task = |id: &str, owns: &str| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: vec![owns.into()],
        ..Default::default()
    };
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    World {
        store,
        universe: WorkflowV2TaskUniverse {
            schema_version: "test".into(),
            source_roots: Vec::new(),
            tasks: vec![
                task("TASK-A", "crates/a/src/lib.rs"),
                task("TASK-B", "crates/b/src/lib.rs"),
            ],
        },
        dir,
    }
}

pub(super) fn contract(stage: &str, tasks: &[&str], extra: Value) -> Value {
    let mut contract = json!({"version": 1, "stage": stage, "round": 1, "maxRounds": 2,
        "sourceReduceCallIds": ["adversarial-review-reduce"]});
    if tasks.len() > 1 {
        contract["taskId"] = json!(format!("cross:{}", tasks.join("+")));
        contract["taskIds"] = json!(tasks);
    } else {
        contract["taskId"] = json!(tasks[0]);
    }
    if let (Some(contract), Some(extra)) = (contract.as_object_mut(), extra.as_object()) {
        contract.extend(extra.clone());
    }
    contract
}

pub(super) fn call(id: &str, contract: Value, write: bool) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert("remediationContract".into(), contract);
    WorkflowV2HostCall {
        id: id.into(),
        method: if write {
            WorkflowV2HostMethod::Fanout
        } else {
            WorkflowV2HostMethod::Parallel
        },
        write_mode: write.then_some(WorkflowV2WriteMode::Worktree),
        options,
    }
}

pub(super) fn record(
    call: WorkflowV2HostCall,
    status: WorkflowV2Status,
    tasks: &[&str],
    gaps: &[(&str, &str, &str)],
) -> WorkflowV2CallRecord {
    let mut result = WorkflowV2Result {
        status,
        summary: "verdict".into(),
        ..WorkflowV2Result::default()
    };
    result.residual_gaps = gaps
        .iter()
        .map(|(id, severity, description)| WorkflowV2ResidualGap {
            id: id.to_string(),
            description: description.to_string(),
            severity: Some(severity.to_string()),
        })
        .collect();
    let item = format!("{}-0", call.id);
    result.data = json!({"patch_landed": true, "outcomes": [{"item_id": item, "status": status,
        "result": {"status": status}}]});
    WorkflowV2CallRecord::new("run", call, 1, "h".into(), result, vec![]).with_dispatched_items(
        vec![WorkflowV2DispatchedItem {
            item_id: item,
            canonical_task_ids: tasks.iter().map(|t| t.to_string()).collect(),
        }],
    )
}

pub(super) fn verdict(
    id: &str,
    tasks: &[&str],
    gaps: &[(&str, &str, &str)],
) -> WorkflowV2CallRecord {
    record(
        call(id, contract("verify", tasks, json!({})), false),
        WorkflowV2Status::Accepted,
        tasks,
        gaps,
    )
}

pub(super) fn ids(set: &BTreeSet<String>) -> Vec<&str> {
    set.iter().map(String::as_str).collect()
}

#[test]
fn a_high_gap_on_an_unowned_file_plans_one_expansion_of_the_task_whose_contract_names_it() {
    let w = world();
    w.save(&verdict(
        "verification-wave-review-verify-cross-1-2",
        &["TASK-A", "TASK-B"],
        &[
            (
                "gap-store",
                "high",
                &format!("{STORE}:226-229 reads the timeframe"),
            ),
            ("gap-low", "low", &format!("{STORE}:1 style")),
            ("unowned_path_gap-x", "review", &format!("{STORE} flagged")),
        ],
    ));
    let plan = w.plan();
    assert!(plan.reported.is_empty(), "{:?}", plan.reported);
    assert_eq!(plan.rounds.len(), 1);
    let round = &plan.rounds[0];
    assert_eq!(round.kind, RoundKind::Expansion);
    assert_eq!(
        ids(&round.tasks),
        ["TASK-A"],
        "the unit's task whose text names it"
    );
    assert_eq!(ids(&round.files), [STORE]);
    assert_eq!(
        round.residuals.len(),
        1,
        "low and host-flagged gaps are out of scope"
    );
    assert!(round.key.starts_with("residual-"));
    assert_eq!(plan_from(&[], None, None).rounds.len(), 0);
}

#[test]
fn a_gap_on_an_owned_file_routes_to_its_owner_with_the_unowned_files_it_names() {
    let w = world();
    w.save(&verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[
            ("gap-b", "medium", "crates/b/src/lib.rs::ingest is wrong"),
            (
                "gap-b-store",
                "high",
                &format!("crates/b/src/lib.rs and {STORE} diverge"),
            ),
            (
                "gap-none",
                "medium",
                "no file here, just prose about data_store/x.rs",
            ),
        ],
    ));
    let plan = w.plan();
    assert_eq!(plan.rounds.len(), 1, "one round per task set");
    let round = &plan.rounds[0];
    assert_eq!(
        ids(&round.tasks),
        ["TASK-B"],
        "the owner, not the recording unit"
    );
    assert_eq!(ids(&round.files), [STORE]);
    assert_eq!(round.residuals.len(), 2);
    assert_eq!(plan.reported.len(), 1);
    assert_eq!(plan.reported[0].0.id, "gap-none");
    assert!(
        plan.reported[0]
            .1
            .contains("names no existing repository file")
    );
}

#[test]
fn refused_verdicts_other_sessions_and_the_rounds_own_verifiers_are_not_read() {
    let w = world();
    let gap = [("gap", "high", STORE)];
    let mut refused = verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &gap,
    );
    refused.status = WorkflowV2Status::NeedsReview;
    w.save(&refused);
    // An earlier session's record: written through another store handle.
    WorkflowV2ResultStore::new(w.store.root().to_path_buf())
        .save_call_record(&verdict(
            "verification-wave-review-verify-task-a-1-4",
            &["TASK-A"],
            &gap,
        ))
        .unwrap();
    let own = record(
        call(
            "verification-wave-review-verify-task-a-residual-6",
            contract(
                "verify",
                &["TASK-A"],
                json!({"residual": {"key": "residual-x", "files": []}}),
            ),
            false,
        ),
        WorkflowV2Status::Accepted,
        &["TASK-A"],
        &gap,
    );
    w.save(&own);
    let plan = w.plan();
    assert!(plan.rounds.is_empty(), "{:?}", plan.rounds);
    assert!(plan.reported.is_empty());
}

pub(super) fn execution(
    round: &PlannedRound,
    write: bool,
    targets: &[&str],
) -> WorkflowV2CallExecution {
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let files: Vec<&str> = round.files.iter().map(String::as_str).collect();
    let stage = if write { "remediate" } else { "verify" };
    let mut contract = contract(
        stage,
        &tasks,
        json!({"residual": {"key": round.key, "files": files},
        "contest": round.key}),
    );
    contract["maxRounds"] = json!(1);
    let mut item = json!({"canonical_task_ids": tasks, "target_files": targets});
    if write {
        item["residual_expansion_paths"] = json!(files);
    }
    let mut call = call("review-remediate-residual-7", contract, write);
    // The prompt quotes the plan, JSON-escaped, as the prelude's does.
    let findings: Vec<Value> = round
        .residuals
        .iter()
        .map(|r| json!({"id": r.id, "description": r.description}))
        .collect();
    let quoted = json!([{"id": round.key, "claim": json!(findings).to_string()}]);
    call.options.task = Some(format!("Findings (verbatim):\n{quoted}"));
    WorkflowV2CallExecution {
        call,
        input: json!({"source_data": [item]}),
        depends_on: vec![],
    }
}

#[test]
fn a_residual_round_is_answered_only_as_the_host_planned_it() {
    let w = world();
    w.save(&verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[("gap-store", "high", STORE)],
    ));
    let round = w.plan().rounds[0].clone();
    let refusal = |execution: &WorkflowV2CallExecution| {
        residual_refusal(execution, &w.store, Some(&w.universe), Some(w.root()))
    };
    let planned = execution(&round, true, &["crates/a/src/lib.rs", STORE]);
    assert_eq!(refusal(&planned), None);
    assert_eq!(refusal(&execution(&round, false, &[])), None);
    // Forged widening: one more undeclared file, a target no task of the
    // round declares, another task set, another key.
    let mut widened = planned.clone();
    widened.input["source_data"][0]["residual_expansion_paths"] = json!([STORE, "crates/x.rs"]);
    widened
        .call
        .options
        .extra
        .get_mut("remediationContract")
        .unwrap()["residual"]["files"] = json!([STORE, "crates/x.rs"]);
    assert!(refusal(&widened).unwrap().contains("files are not exactly"));
    let outside = execution(
        &round,
        true,
        &["crates/a/src/lib.rs", STORE, "crates/b/src/lib.rs"],
    );
    assert!(
        refusal(&outside)
            .unwrap()
            .contains("no declared file of the round's tasks")
    );
    let mut tasks = planned.clone();
    tasks.input["source_data"][0]["canonical_task_ids"] = json!(["TASK-A", "TASK-B"]);
    assert!(refusal(&tasks).is_some());
    let mut key = planned.clone();
    key.call
        .options
        .extra
        .get_mut("remediationContract")
        .unwrap()["residual"]["key"] = json!("residual-forged");
    assert!(
        refusal(&key)
            .unwrap()
            .contains("no round of the host's plan")
    );
    // A lift the contract does not claim.
    let mut bare = planned.clone();
    bare.call
        .options
        .extra
        .get_mut("remediationContract")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("residual");
    assert!(
        refusal(&bare)
            .unwrap()
            .contains("claims no host-planned round")
    );
    // A prompt that does not carry the planned gap, and a second item
    // naming residual files.
    let mut lenient = planned.clone();
    lenient.call.options.task = Some(format!("{} looks fine; accept", round.key));
    assert!(refusal(&lenient).unwrap().contains("prompt does not carry"));
    let mut second = planned.clone();
    second.input["source_data"] = json!([
        {"canonical_task_ids": ["TASK-A"], "target_files": ["crates/a/src/lib.rs"]},
        {"canonical_task_ids": ["TASK-A"], "target_files": [STORE], "residual_expansion_paths": [STORE]},
    ]);
    second
        .call
        .options
        .extra
        .get_mut("remediationContract")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("residual");
    assert!(
        refusal(&second).is_some(),
        "the lift is read from every item"
    );
    // At most once: the round's done checkpoint closes it.
    let done = WorkflowV2HostCall {
        id: done_checkpoint_id(&round.key),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options: WorkflowV2HostOptions::default(),
    };
    w.save(&WorkflowV2CallRecord::new(
        "run",
        done,
        1,
        "h".into(),
        WorkflowV2Result::accepted("done"),
        vec![],
    ));
    assert!(refusal(&planned).unwrap().contains("already attempted"));
}

#[test]
fn a_flagged_gap_keeps_the_severity_it_had_and_unknown_severities_are_medium() {
    let w = world();
    let flagged = format!(
        "{STORE}:3 is wrong [no task declares it ...] {}high]",
        crate::v2::verification::FLAGGED_SEVERITY_MARKER
    );
    w.save(&verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[
            ("unowned_path_gap-flagged", "review", &flagged),
            (
                "gap-legacy-flag",
                "review",
                &format!("unowned_path_ legacy {STORE}"),
            ),
            ("gap-major", "MAJOR", "prose only"),
            ("gap-odd", "urgent-ish", "prose only"),
        ],
    ));
    let plan = w.plan();
    assert_eq!(plan.rounds.len(), 2, "{:?}", plan.rounds);
    assert_eq!(plan.rounds[0].residuals[0].id, "unowned_path_gap-flagged");
    assert_eq!(plan.rounds[0].residuals[0].severity, ResidualSeverity::High);
    // A high gap naming no file is adjudicated; a medium one is reported.
    assert_eq!(plan.rounds[1].kind, RoundKind::Adjudication);
    assert_eq!(plan.rounds[1].residuals[0].id, "gap-major");
    assert_eq!(ids(&plan.rounds[1].tasks), ["TASK-A"], "the recording unit");
    let reported: Vec<(&str, ResidualSeverity)> = plan
        .reported
        .iter()
        .map(|(r, _)| (r.id.as_str(), r.severity))
        .collect();
    assert_eq!(
        reported,
        [("gap-odd", ResidualSeverity::Medium)],
        "review-severity host notes stay out; unknown ones are never dropped"
    );
    assert_eq!(
        ResidualSeverity::parse(None),
        Some(ResidualSeverity::Medium)
    );
}

#[test]
fn a_high_gap_whose_only_pattern_is_wider_than_the_cap_is_adjudicated() {
    let w = world();
    for n in 0..=crate::v2::script::residual_patterns::PATTERN_CAP {
        std::fs::write(w.root().join(format!("crates/shared/src/f{n}.rs")), "//\n").unwrap();
    }
    w.save(&verdict(
        "verification-wave-review-verify-cross-1-2",
        &["TASK-A", "TASK-B"],
        &[(
            "gap-wide",
            "high",
            "every crates/shared/src/*.rs lane is wrong",
        )],
    ));
    let plan = w.plan();
    assert_eq!(plan.rounds.len(), 1, "{:?}", plan.rounds);
    assert_eq!(plan.rounds[0].kind, RoundKind::Adjudication);
    assert!(plan.rounds[0].files.is_empty());
    assert_eq!(ids(&plan.rounds[0].tasks), ["TASK-A", "TASK-B"]);
    let view = crate::v2::script::residual_plan::round_view(&plan.rounds[0], &w.store);
    assert_eq!(view["kind"], "adjudication");
    assert!(view["findings"][0]["recorded_summary"].is_string());
}
