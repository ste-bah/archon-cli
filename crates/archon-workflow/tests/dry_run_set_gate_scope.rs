//! Batch O: the set gate's coverage and scope reading of a live task set,
//! without a run. Ignored by default; run with
//!   ARCHON_DRY_RUN_RUN=<run dir> ARCHON_DRY_RUN_REPO=<target repository>
//!   [ARCHON_DRY_RUN_COPY=<copy of the run dir: its records' landings, and
//!   the scope amendments a remediation-plan dry run recorded there>]
//!   [ARCHON_DRY_RUN_EXPECT=<json: {restore: {task: [file]}, owners: [file]}>]
//!   cargo test -p archon-workflow --test dry_run_set_gate_scope -- --ignored --nocapture
//!
//! It prints the PRD requirement ids the frozen acceptance contract covers
//! with no check (each owed a supplementary check), and the scope
//! amendments the host would record: declared files the authored script
//! left out, restored, and every load-bearing file no task declares -- one a
//! landing touched or a review finding named -- assigned to a task. It
//! writes nothing.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use archon_workflow::task_scope_amendment::{
    ScopeAmendment, ScopeGrantKind, ScopePlanInputs, plan_scope_amendments,
};
use archon_workflow::task_set_contract::AcceptanceContract;
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::acceptance_stage::coverage::{
    prd_requirement_ids, uncovered_requirements,
};
use archon_workflow::v2::script::remediation_plan::{finding_text, task_scope_of};
use archon_workflow::*;
use serde_json::Value;

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// The authored script's task table, evaluated.
fn authored_tasks(script: &str) -> Vec<Value> {
    let start = script.find("const tasks = [").unwrap();
    let end = start + script[start..].find("\n]\n").unwrap() + 3;
    let runtime = rquickjs::Runtime::new().unwrap();
    let context = rquickjs::Context::full(&runtime).unwrap();
    let json: String = context.with(|ctx| {
        ctx.eval(format!(
            "(function() {{ {} return JSON.stringify(tasks); }})()",
            &script[start..end]
        ))
        .unwrap()
    });
    serde_json::from_str(&json).unwrap()
}

#[test]
#[ignore = "needs a live task set: ARCHON_DRY_RUN_RUN and ARCHON_DRY_RUN_REPO"]
fn the_set_gate_reads_coverage_and_scope_of_a_live_task_set() {
    let (Some(run), Some(repo)) = (env("ARCHON_DRY_RUN_RUN"), env("ARCHON_DRY_RUN_REPO")) else {
        eprintln!("ARCHON_DRY_RUN_RUN / ARCHON_DRY_RUN_REPO unset; nothing to do");
        return;
    };
    let read =
        |path: PathBuf| -> Value { serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap() };
    let metadata = read(run.join("v2/generated-metadata.json"));
    let universe: WorkflowV2TaskUniverse =
        serde_json::from_value(metadata["task_universe"].clone()).unwrap();
    let tasks_root = PathBuf::from(&universe.source_roots[0]);
    let contract: AcceptanceContract = serde_json::from_slice(
        &std::fs::read(tasks_root.join("acceptance-contract.json")).unwrap(),
    )
    .unwrap();
    let prd_path = tasks_root
        .ancestors()
        .map(|dir| dir.join(&contract.prd.path))
        .find(|p| p.is_file())
        .expect("the PRD the contract names");
    let prd = std::fs::read_to_string(&prd_path).unwrap();

    // Coverage: every PRD requirement no check covers is owed a check.
    let ids = prd_requirement_ids(&prd);
    let uncovered = uncovered_requirements(&ids, &contract);
    println!(
        "== PRD requirements {} | checks {} | uncovered {}",
        ids.len(),
        contract.acceptance.len() + contract.supplementary.len(),
        uncovered.len()
    );
    println!("uncovered: {}", uncovered.join(", "));

    // Scope: the host's amendments.
    let script = std::fs::read_to_string(run.join("authored-workflow.js")).unwrap();
    let authored: BTreeMap<String, BTreeSet<String>> = authored_tasks(&script)
        .into_iter()
        .map(|t| {
            let files = ["targetFiles", "artifacts"]
                .iter()
                .flat_map(|k| t[*k].as_array().cloned().unwrap_or_default())
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            (t["id"].as_str().unwrap().to_string(), files)
        })
        .collect();
    let result = read(run.join("v2/script-result.json"));
    let named: Vec<BTreeSet<String>> = ["adversarial_findings", "uncovered_requirements"]
        .iter()
        .flat_map(|f| result[*f].as_array().cloned().unwrap_or_default())
        .map(|finding| {
            archon_workflow::v2::script::remediation_plan::explicitly_named(
                &finding_text(&finding),
                &repo,
            )
            .into_iter()
            .collect()
        })
        .collect();
    let landed = env("ARCHON_DRY_RUN_COPY")
        .map(|copy| {
            archon_workflow::v2::script::remediation_plan::landed_files_by_task(
                &WorkflowV2ResultStore::new(copy.join("v2")),
                None,
            )
        })
        .unwrap_or_default();
    let empty = BTreeMap::new();
    let plan = plan_scope_amendments(&ScopePlanInputs {
        universe: &universe,
        repository_root: &repo,
        project_root: None,
        authored: &authored,
        landed_files_by_task: &landed,
        finding_named_files: &named,
        focused_test_files_by_task: &empty,
    });
    println!(
        "== amendments {} | unassigned {}",
        plan.amendments.len(),
        plan.unassigned.len()
    );
    for grant in &plan.amendments {
        println!(
            "grant {:?} {} -> {} ({})",
            grant.kind, grant.path, grant.task_id, grant.evidence
        );
    }
    for (file, why) in &plan.unassigned {
        println!("unassigned {file}: {why}");
    }
    // How broad the grants are: per task, per rule, and how deep the code
    // tier reached.
    let mut per_task: BTreeMap<&str, usize> = BTreeMap::new();
    let mut per_rule: BTreeMap<&str, usize> = BTreeMap::new();
    for grant in &plan.amendments {
        *per_task.entry(grant.task_id.as_str()).or_default() += 1;
        *per_rule.entry(grant.evidence.as_str()).or_default() += 1;
    }
    println!("== grants per task: {per_task:?}");
    println!("== grants per rule: {per_rule:?}");
    // Ownership is not write scope: every ownerless assignment is an owner
    // record, except a file the task's own landing changed, which stays
    // writable to it; a task's dispatch scope is what it declares, what the
    // script authored for it, its declared-file restores and those landings.
    let stray: Vec<&ScopeAmendment> = plan
        .amendments
        .iter()
        .filter(|g| {
            g.kind.writable()
                && g.kind != ScopeGrantKind::DeclaredRestore
                && g.evidence != "a landing of the task changed it"
        })
        .collect();
    assert!(
        stray.is_empty(),
        "the set gate grants write scope: {stray:?}"
    );
    let owner_per_task = |task: &str| {
        plan.amendments
            .iter()
            .filter(|g| g.task_id == task && g.kind == ScopeGrantKind::Owner)
            .count()
    };
    let writable = |task: &str| -> BTreeSet<String> {
        let mut files: BTreeSet<String> =
            task_scope_of(&universe, task, &repo).into_iter().collect();
        files.extend(authored.get(task).into_iter().flatten().cloned());
        files.extend(
            plan.amendments
                .iter()
                .filter(|g| g.task_id == task && g.kind.writable())
                .map(|g| g.path.clone()),
        );
        files
    };
    let ids: Vec<&str> = universe
        .tasks
        .iter()
        .map(|t| t.canonical_task_id.as_str())
        .collect();
    let owners_vs_writable: BTreeMap<&str, (usize, usize)> = ids
        .iter()
        .map(|task| (*task, (owner_per_task(task), writable(task).len())))
        .collect();
    println!("== per task (owner records, dispatch-writable files): {owners_vs_writable:?}");
    let usage = archon_workflow::task_scope_amendment::refs::code_usage(&universe, &repo);
    let mut depths: BTreeMap<usize, usize> = BTreeMap::new();
    for depth in usage.depth.values() {
        *depths.entry(*depth).or_default() += 1;
    }
    println!(
        "== code tier: {} file(s) owned; files per reference depth {depths:?}; max depth {}",
        usage.owners.len(),
        depths.keys().max().copied().unwrap_or(0)
    );
    let dead = plan
        .unassigned
        .iter()
        .filter(|(_, why)| why.starts_with("dead code"))
        .count();
    println!("== unassigned reported as dead code: {dead}");
    if let Some(expect) = env("ARCHON_DRY_RUN_EXPECT") {
        let expect = read(expect);
        let granted = |task: &str, file: &str| {
            plan.amendments
                .iter()
                .any(|g| g.task_id == task && g.path == file)
        };
        let mut ok = true;
        for (task, files) in expect["restore"].as_object().into_iter().flatten() {
            for file in files
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let hit = granted(task, file);
                ok &= hit;
                println!(
                    "expect restore {file} -> {task}: {}",
                    if hit { "YES" } else { "NO" }
                );
            }
        }
        // The remediation plan's own recorded amendments (a finding of a
        // task naming a file no task declares), when the copy holds them.
        let ledger = env("ARCHON_DRY_RUN_COPY").and_then(|copy| {
            archon_workflow::task_scope_amendment::ScopeAmendmentLedger::load(&copy).ok()
        });
        for file in expect["owners"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let by_gate: Vec<&str> = plan
                .amendments
                .iter()
                .filter(|g| g.path == file)
                .map(|g| g.task_id.as_str())
                .collect();
            let by_plan: Vec<String> = ledger
                .iter()
                .flat_map(|l| l.set.grants.iter())
                .filter(|g| g.path == file)
                .map(|g| format!("{} {:?} ({})", g.task_id, g.kind, g.evidence))
                .collect();
            // The set gate itself must give every expected file an owner.
            ok &= !by_gate.is_empty();
            let dispatch: Vec<&str> = ids
                .iter()
                .copied()
                .filter(|task| writable(task).contains(file))
                .collect();
            println!(
                "expect owner for {file}: set gate {by_gate:?}; writable at dispatch for {dispatch:?}; remediation plan ledger {by_plan:?}"
            );
        }
        assert!(ok, "an expected amendment is missing");
    }
}
