//! R17 admission cells. Run where the existing loopback provider is permitted.
#![allow(dead_code)]
#[path = "support/workflow_decomposition_offline_r17_assertions.rs"]
mod assertions;
#[path = "support/workflow_decomposition_offline_deadline.rs"]
mod deadline;
#[path = "support/workflow_decomposition_offline_diagnostics.rs"]
mod diagnostics;
#[path = "support/workflow_decomposition_offline_r17_fixture_tests.rs"]
mod fixture_tests;
#[path = "support/workflow_decomposition_offline_project.rs"]
mod project;
#[path = "support/workflow_decomposition_offline_r17_resume.rs"]
mod resume_cells;
#[path = "support/workflow_decomposition_offline_server.rs"]
mod server;
#[path = "support/workflow_decomposition_offline_snapshot.rs"]
mod snapshot;

use assertions::*;
use serde_json::{Value, json};
use server::variants::r17::Cell;
use std::fs;

#[test]
fn v0_golden_completion_and_reverify() {
    let p = fixture(Cell::V0);
    let base = std::process::Command::new("git")
        .args([
            "-C",
            p.path().join("repository").to_str().unwrap(),
            "rev-parse",
            "HEAD",
        ])
        .output()
        .unwrap();
    success(&base);
    let base = String::from_utf8(base.stdout).unwrap().trim().to_owned();
    success(&deadline::run(p.path(), &project::decompose_args()));
    shared(&p, 1, [1, 1], 0);
    assert_clean_set_rounds(&p, 1);
    let s = project::final_snapshot(&p, &[]);
    for task in s.skeleton["tasks"].as_array().unwrap() {
        assert!(!task["source_sections"].as_array().unwrap().is_empty());
        assert!(
            task["implements"]
                .as_array()
                .unwrap()
                .contains(&json!("C-X-001"))
        );
        assert!(
            task["implements"]
                .as_array()
                .unwrap()
                .contains(&json!("C-X-003"))
        );
    }
    assert!(
        s.skeleton["tasks"][1]["implements"]
            .as_array()
            .unwrap()
            .contains(&json!("C-X-002"))
    );
    let rows = events(&p);
    assert!(rows.iter().any(|e| e["kind"] == "decomposition_completed"));
    let records = records(&p);
    let report = records
        .iter()
        .find(|r| r["call"]["method"] == "finalReport")
        .expect("run-end report");
    assert_eq!(report["result"]["status"], "accepted", "{report}");
    let pin = read(&pin_path(p.path()));
    assert!(
        pin["skeleton_digest"]
            .as_str()
            .is_some_and(|d| !d.is_empty())
    );
    let output = deadline::run(
        p.path(),
        &["workflow", "reverify", &project::run_id(p.path())],
    );
    success(&output);
    // P2.23: reverify's recorded baseline is the fixture's pre-decomposition commit.
    let tasks = p.path().join("tasks/PRD-X");
    assert_eq!(read(&tasks.join("repository.lock"))["base_commit"], base);
    assert_eq!(
        read(&tasks.join("acceptance-contract.lock"))["baseline_commit"],
        base
    );
    let reports = project::one_run(p.path()).join("decomposition/reverify/reports");
    let reports: Vec<Value> = fs::read_dir(reports)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .map(|p| read(&p))
        .collect();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["outcome"], "accepted");
    for key in ["subjects", "set_gates", "frozen_gates", "checks"] {
        for item in reports[0][key].as_array().unwrap() {
            assert!(
                item["findings"].as_array().is_none_or(Vec::is_empty),
                "{item}"
            );
        }
    }
}

#[test]
fn v1_skeleton_ownership_citation_duplicate_then_freezes() {
    // The rejected duplicate is healed in-session before the first freeze.
    let p = completed(Cell::V1);
    shared_with_skeleton_exchanges(&p, 2, [1, 1], 0, 6);
    let requests = p.provider.requests.snapshot();
    let rejection = requests
        .iter()
        .filter(|r| r.role == "skeleton")
        .flat_map(|r| {
            let request: Value = serde_json::from_str(&r.prompt).unwrap();
            request["messages"].as_array().unwrap().clone()
        })
        .flat_map(|message| message["content"].as_array().cloned().unwrap_or_default())
        .find(|block| {
            block["type"] == "tool_result"
                && block["is_error"] == true
                && block["content"]
                    .as_str()
                    .is_some_and(|text| text.contains("lists '2' twice"))
        })
        .expect("duplicate landing must return an error to the author");
    let text = rejection["content"].as_str().unwrap();
    for expected in ["TASK-X-010", "source_sections", "lists '2' twice"] {
        assert!(text.contains(expected), "{text}");
    }

    let freezes = gates(&p, "freeze-skeleton");
    assert_eq!(
        freezes.len(),
        2,
        "landing must reach freeze on both attempts"
    );
    let first = findings(&freezes[0]);
    let codes: std::collections::BTreeSet<_> = first
        .iter()
        .map(|f| f["deterministic_defect"]["code"].as_str().unwrap())
        .collect();
    assert_eq!(first.len(), 2, "{first:?}");
    assert_eq!(codes, ["uncited_section", "unowned_constraint"].into());
    assert!(first.iter().any(
        |f| f["deterministic_defect"]["code"] == "unowned_constraint"
            && f["deterministic_defect"]["subject"] == "C-X-002"
    ));
    assert!(
        first
            .iter()
            .any(|f| f["deterministic_defect"]["code"] == "uncited_section"
                && f["text"].as_str().unwrap().contains("Closing statement"))
    );
    assert!(
        first
            .iter()
            .any(|f| f["deterministic_defect"]["code"] == "uncited_section"
                && f["deterministic_defect"]["subject"] == "9")
    );
    assert!(findings(&freezes[1]).is_empty());
    assert_clean_set_rounds(&p, 1);
}

#[test]
fn v2_yaml_only_constraint_repaired_at_body_landing() {
    let p = completed(Cell::V2);
    shared(&p, 1, [2, 1], 0);
    let landings = gates(&p, "land-task-body");
    assert_eq!(landings.len(), 3);
    let first = findings(&landings[0]);
    assert!(!first.is_empty());
    assert!(
        first
            .iter()
            .any(|f| f["text"].as_str().unwrap().contains("C-X-001")
                && f["remediation_scope"] == "body"),
        "{first:?}"
    );
    assert!(findings(&landings[1]).is_empty());
    assert_clean_set_rounds(&p, 1);
}

#[test]
fn v3_landing_records_preserve_all_record_binding_fields() {
    let p = completed(Cell::V3);
    shared(&p, 1, [1, 1], 0);
    let candidate = landed_candidates(&p).remove(0);
    let frozen = project::final_snapshot(&p, &[]).skeleton;
    // Compare through the production serde shape so omitted defaults are equal.
    let parse = |v: Value| {
        serde_json::from_value::<archon_workflow::task_skeleton::TaskSkeleton>(v).unwrap()
    };
    let mut expected = parse(candidate);
    for task in &mut expected.tasks {
        assert!(task.requirement_states.is_empty());
        assert!(task.proven_done.is_empty());
        assert!(task.status.is_none());
        // Author-owned bindings remain identical; the host alone adds these states.
        for id in &task.implements {
            let (state, reason) = if ["C-X-001", "C-X-002", "C-X-003"].contains(&id.as_str()) {
                (
                    archon_workflow::requirement_state::RequirementState::Inspection,
                    "inspection_pending",
                )
            } else {
                (
                    archon_workflow::requirement_state::RequirementState::Missing,
                    "all_checks_fail_on_base",
                )
            };
            task.requirement_states.insert(
                id.clone(),
                archon_workflow::requirement_state::RequirementDecision::new(state, reason),
            );
        }
    }
    assert_eq!(expected.tasks, parse(frozen).tasks);
    let requests = p.provider.requests.snapshot();
    let first = requests.iter().find(|r| r.role == "skeleton").unwrap();
    let request: Value = serde_json::from_str(&first.prompt).unwrap();
    let schema = request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|t| t.pointer("/input_schema/properties/task/properties"))
        .unwrap();
    for key in [
        "task_id",
        "file_name",
        "depends_on",
        "blocks",
        "implements",
        "source_sections",
        "deliverable_contracts",
    ] {
        assert!(!schema[key].is_null(), "schema omits {key}");
    }
    assert!(
        requests
            .iter()
            .filter(|r| r.role == "skeleton")
            .all(|r| !r.answer.contains("records_landed"))
    );
    let author = records(&p)
        .into_iter()
        .find(|r| {
            r["call"]["method"] == "agent"
                && r["call"]["id"]
                    .as_str()
                    .unwrap()
                    .starts_with("skeleton-author-")
        })
        .unwrap();
    assert_eq!(author["result"]["data"]["toolTrace"]["toolCalls"], 2);
    assert_eq!(author["result"]["data"]["toolTrace"]["complete"], true);
    assert_clean_set_rounds(&p, 1);
}

#[test]
fn v4_truncated_set_critic_retries_without_skeleton_author() {
    // An output-limit truncation with a stalled continuation is unusable,
    // so P6.11 requires an observable gate retry, without author churn.
    let p = completed(Cell::V4);
    shared(&p, 1, [1, 1], 0);
    let trace = gates(&p, "task-set-lint");
    assert_eq!(
        trace.len(),
        2,
        "the set gate, rather than the skeleton, is retried"
    );
    assert!(trace[0].to_string().contains("authored_judge_reply"));
    assert!(findings(&trace[1]).is_empty());
    let requests = p.provider.requests.snapshot();
    let body_end = requests.iter().rposition(|r| r.role == "body").unwrap();
    assert!(
        requests[body_end + 1..]
            .iter()
            .all(|r| r.role != "skeleton")
    );
}

#[test]
fn v5_set_skeleton_finding_patches_only_named_record_and_reached_body() {
    // Quote-less ownership evidence must reach the skeleton patch author.
    let p = completed(Cell::V5);
    shared(&p, 2, [1, 2], 0);
    let set = gates(&p, "task-set-lint");
    assert_eq!(set.len(), 2);
    let first = findings(&set[0]);
    assert_eq!(first.len(), 1);
    assert_eq!(
        first[0]["remediation_scope"], "skeleton",
        "critic must expose a skeleton-scope finding"
    );
    let candidates = landed_candidates(&p);
    assert_eq!(candidates.len(), 2);
    assert_eq!(
        serde_json::to_vec(&candidates[0]["tasks"][0]).unwrap(),
        serde_json::to_vec(&candidates[1]["tasks"][0]).unwrap()
    );
    let requests = p.provider.requests.snapshot();
    let patch: Value = serde_json::from_str(
        &requests
            .iter()
            .filter(|r| r.role == "skeleton")
            .nth(2)
            .unwrap()
            .answer,
    )
    .unwrap();
    assert_eq!(patch["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(patch["tasks"][0]["task_id"], "TASK-X-020");
    assert!(findings(&set[1]).is_empty());
}

#[test]
fn v6_unparseable_body_retries_without_skeleton_findings() {
    let p = completed(Cell::V6);
    shared(&p, 1, [2, 1], 0);
    let landings = gates(&p, "land-task-body");
    assert_eq!(landings.len(), 3);
    assert!(!findings(&landings[0]).is_empty());
    assert!(findings(&landings[1]).is_empty());
    for gate in gates(&p, "task-set-lint") {
        assert!(
            findings(&gate)
                .iter()
                .all(|f| f["remediation_scope"] != "skeleton")
        );
    }
    // Also lint the malformed body directly while a skeleton exists, since
    // the lifecycle repairs it before its set gate normally sees it.
    let path = p.path().join("tasks/PRD-X/TASK-X-010.md");
    let good = fs::read_to_string(&path).unwrap();
    fs::write(&path, good.replacen("\n```\n", "\n", 1)).unwrap();
    let envelope = p.path().join("malformed-envelope.json");
    deadline::run(
        p.path(),
        &[
            "workflow",
            "lint",
            "--tasks",
            "tasks/PRD-X",
            "--gate-envelope",
            envelope.to_str().unwrap(),
            "--call-id",
            "r17-malformed",
        ],
    );
    let envelope = read(&envelope);
    let fs = envelope["policy_findings"].as_array().unwrap();
    assert!(!fs.is_empty());
    assert!(
        fs.iter().all(|f| f["remediation_scope"] != "skeleton"),
        "{fs:?}"
    );
    fs::write(path, good).unwrap();
}
