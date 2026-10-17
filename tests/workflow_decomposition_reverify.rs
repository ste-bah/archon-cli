#![allow(dead_code)]

#[path = "support/workflow_decomposition_offline_deadline.rs"]
mod deadline;
#[path = "support/workflow_decomposition_offline_diagnostics.rs"]
mod diagnostics;
#[path = "support/workflow_decomposition_offline_faults.rs"]
mod faults;
#[path = "support/workflow_decomposition_offline_project.rs"]
mod project;
#[path = "support/workflow_decomposition_offline_server.rs"]
mod server;
#[path = "support/workflow_decomposition_offline_snapshot.rs"]
mod snapshot;
#[path = "support/workflow_decomposition_offline_variant_cells.rs"]
mod variant_cells;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use deadline::run;
use project::{decompose_args, one_run, project_with};
use server::{Plan, Variant};

fn record_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let directory = root.join("v2/results");
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (path.clone(), fs::read(path).unwrap())
        })
        .collect()
}

fn newest_report(run: &Path) -> (PathBuf, serde_json::Value) {
    let reports = run.join("decomposition/reverify/reports");
    let path = fs::read_dir(reports)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .max_by_key(|path| path.file_name().unwrap().to_os_string())
        .unwrap();
    let value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    (path, value)
}

fn tree_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(tree_bytes(&path));
        } else if path.is_file() {
            files.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    files
}

#[test]
fn reverify_uses_local_provider_for_accepted_and_real_finding_paths() {
    // EXPECTED BEFORE RUN: the real offline lifecycle reaches Completed using
    // the local provider; its frozen copy contains `../shared-check` and the
    // reverify report records that sibling's source HEAD. Reverify accepts
    // disk bytes with fresh digests,
    // current stamps, a superseding report, and byte-identical old records.
    // Two defects pause the run; dry-run is read-only; resume reauthors only
    // the ledger subjects and returns an ordinary accepted final report.
    let project = project_with(Plan {
        variant: Variant::SiblingCheck,
        block: None,
    });
    let root = project.path();
    let completed = run(root, &decompose_args());
    assert!(
        completed.status.success(),
        "{}",
        String::from_utf8_lossy(&completed.stderr)
    );
    let run_dir = one_run(root);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().into_owned();
    let original_state = fs::read(run_dir.join("state.json")).unwrap();
    let original_records = record_bytes(&run_dir);
    let bodies = ["TASK-X-010.md", "TASK-X-020.md"]
        .into_iter()
        .map(|name| root.join("tasks/PRD-X").join(name))
        .collect::<Vec<_>>();
    let body_bytes = bodies
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect::<Vec<_>>();
    println!(
        "EVIDENCE reverify accepted BEFORE: run=Completed body_count={} prior_records={} prior_report=golden-final-report",
        bodies.len(),
        original_records.len()
    );

    let accepted = run(root, &["workflow", "reverify", &run_id]);
    assert!(
        accepted.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&accepted.stdout),
        String::from_utf8_lossy(&accepted.stderr)
    );
    let gate_progress = String::from_utf8_lossy(&accepted.stderr);
    for (subject, capability) in [
        ("task-set", "task-set-lint"),
        ("task-set", "requirements-trace"),
        ("frozen-task-set", "verify-frozen-acceptance"),
        ("frozen-task-set", "verify-frozen-skeleton"),
    ] {
        assert_eq!(
            gate_progress
                .matches(&format!(
                    "reverify gate started subject={subject} capability={capability}"
                ))
                .count(),
            1,
            "missing unique gate start progress for {subject}/{capability}: {gate_progress}"
        );
        assert_eq!(
            gate_progress
                .matches(&format!(
                    "reverify gate finished subject={subject} capability={capability}"
                ))
                .count(),
            1,
            "missing unique gate finish progress for {subject}/{capability}: {gate_progress}"
        );
    }
    let (accepted_path, accepted_report) = newest_report(&run_dir);
    assert_eq!(accepted_report["outcome"], "accepted");
    let checks = accepted_report["checks"].as_array().unwrap();
    assert!(
        !checks.is_empty(),
        "frozen checks were not re-run: {accepted_report}"
    );
    let sibling_check = checks
        .iter()
        .find(|check| check["check_id"] == "AC-X-001")
        .unwrap();
    assert_eq!(sibling_check["verdict"], "accepted");
    assert_eq!(sibling_check["really_ran"], true);
    let sibling = sibling_check["sibling_paths"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sibling| sibling["path"].as_str().unwrap().contains("shared-check"))
        .expect("the freeze copy records the named sibling");
    assert_eq!(sibling["path"], "../shared-check");
    let source_sibling = root.parent().unwrap().join("shared-check");
    assert!(source_sibling.join("marker.txt").is_file());
    let source_head = std::process::Command::new("git")
        .arg("-C")
        .arg(&source_sibling)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(source_head.status.success());
    assert_eq!(
        sibling["commit"].as_str(),
        Some(String::from_utf8_lossy(&source_head.stdout).trim())
    );
    assert_eq!(
        accepted_report["supersedes_id"], "fixed-decomposition-final",
        "reverify must supersede the completed lifecycle report"
    );
    for (subject, path) in accepted_report["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&bodies)
    {
        let disk = fs::read(path).unwrap();
        assert_eq!(
            subject["disk_digest"],
            archon_workflow::task_set_contract::content_digest(&disk)
        );
        assert_eq!(subject["judged_digest"], subject["disk_digest"]);
        assert_eq!(subject["verdict"], "accepted");
    }
    for gate in accepted_report["set_gates"]
        .as_array()
        .unwrap()
        .iter()
        .chain(accepted_report["frozen_gates"].as_array().unwrap())
    {
        assert!(gate["logic_version"].as_u64().is_some());
        assert!(gate["logic_digest"].as_str().is_some());
        assert!(gate["build"].as_str().is_some());
        assert_eq!(gate["verdict"], "accepted");
    }
    let reverify_records = archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2"))
        .load_call_records()
        .unwrap();
    let gate_records = reverify_records
        .iter()
        .filter(|record| {
            record.call.id.starts_with("reverify-") && record.call.id.contains("-gate-")
        })
        .collect::<Vec<_>>();
    assert_eq!(gate_records.len(), 4);
    for record in gate_records {
        assert_eq!(
            record.result.data["observedOnly"], true,
            "{}",
            record.call.id
        );
        assert!(record.result.data.get("publicationReceipt").is_none());
    }
    assert_eq!(
        fs::read(run_dir.join("state.json")).unwrap(),
        original_state
    );
    assert_eq!(
        bodies
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>(),
        body_bytes
    );
    let accepted_records = record_bytes(&run_dir);
    for (path, bytes) in &original_records {
        assert_eq!(
            accepted_records.get(path),
            Some(bytes),
            "prior record changed: {}",
            path.display()
        );
    }
    println!(
        "EVIDENCE reverify accepted AFTER: outcome={} report={} supersedes={} digest_checks={} gate_stamps=current run=Completed prior_records_byte_identical=true",
        accepted_report["outcome"],
        accepted_path.file_name().unwrap().to_string_lossy(),
        accepted_report["supersedes_id"],
        bodies.len()
    );

    let defective = b"not a task body\n";
    fs::write(&bodies[0], defective).unwrap();
    let well_formed_body = fs::read(&bodies[1]).unwrap();
    let well_formed_text = String::from_utf8(well_formed_body.clone()).unwrap();
    assert_eq!(
        well_formed_text
            .matches("implements:\n- REQ-X-002\n")
            .count(),
        1,
        "TASK-X-020 should carry REQ-X-002 in its implements list"
    );
    let missing_obligation =
        well_formed_text.replace("implements:\n- REQ-X-002\n", "implements: []\n");
    assert_ne!(
        missing_obligation.as_bytes(),
        well_formed_body.as_slice(),
        "removing TASK-X-020's REQ-X-002 must change the fixture bytes"
    );
    assert_ne!(
        missing_obligation,
        String::from_utf8(body_bytes[1].clone()).unwrap(),
        "the modified body should differ from the accepted body"
    );
    fs::write(&bodies[1], missing_obligation.as_bytes()).unwrap();
    let before_state = fs::read(run_dir.join("state.json")).unwrap();
    let prior_ledger = run_dir.join(format!(
        "decomposition/reverify/remediation-{}.json",
        accepted_report["id"].as_str().unwrap()
    ));
    let prior_ledger_bytes = prior_ledger
        .exists()
        .then(|| fs::read(&prior_ledger).unwrap());
    println!(
        "EVIDENCE reverify finding BEFORE: malformed=TASK-X-010 bytes={} missing_obligation=TASK-X-020 well_formed=true run=Completed ledger_present={}",
        defective.len(),
        prior_ledger.exists()
    );
    let finding = run(root, &["workflow", "reverify", &run_id]);
    assert!(
        finding.status.success(),
        "{}",
        String::from_utf8_lossy(&finding.stderr)
    );
    let (finding_path, finding_report) = newest_report(&run_dir);
    assert_eq!(finding_report["outcome"], "needs_remediation");
    assert_eq!(finding_report["supersedes_id"], accepted_report["id"]);
    let subject = finding_report["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|subject| subject["task_id"] == "TASK-X-010")
        .unwrap();
    assert_eq!(subject["verdict"], "needs_remediation");
    assert!(!subject["findings"].as_array().unwrap().is_empty());
    let second_subject = finding_report["subjects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|subject| subject["task_id"] == "TASK-X-020")
        .unwrap();
    assert_eq!(second_subject["verdict"], "needs_remediation");
    assert!(!second_subject["findings"].as_array().unwrap().is_empty());
    let status: serde_json::Value =
        serde_json::from_slice(&fs::read(run_dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(status["status"], "paused");
    assert_ne!(fs::read(run_dir.join("state.json")).unwrap(), before_state);
    let latest_ledger = run_dir.join(format!(
        "decomposition/reverify/remediation-{}.json",
        finding_report["id"].as_str().unwrap()
    ));
    assert!(latest_ledger.is_file(), "latest report ledger missing");
    if let Some(prior_bytes) = prior_ledger_bytes {
        assert_eq!(fs::read(&prior_ledger).unwrap(), prior_bytes);
    }
    let ledger_bytes = fs::read(&latest_ledger).unwrap();
    let ledger: serde_json::Value = serde_json::from_slice(&ledger_bytes).unwrap();
    assert!(
        ledger["subjects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == "TASK-X-010")
    );
    assert!(
        ledger["subjects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == "TASK-X-020")
    );
    assert_eq!(ledger["report_id"], finding_report["id"]);
    assert_eq!(fs::read(&bodies[0]).unwrap(), defective);
    assert_eq!(fs::read(&bodies[1]).unwrap(), missing_obligation.as_bytes());
    let finding_stdout = String::from_utf8_lossy(&finding.stdout);
    assert!(
        finding_stdout.contains(&format!(
            "Resume: archon workflow resume --live --yes {run_id}"
        )),
        "{finding_stdout}"
    );
    let run_bytes_before_dry_run = tree_bytes(&run_dir);
    let state_before_dry_run = fs::read(run_dir.join("state.json")).unwrap();
    let ledger_before_dry_run = fs::read(&latest_ledger).unwrap();
    let bodies_before_dry_run = bodies
        .iter()
        .map(fs::read)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let results_before_dry_run = record_bytes(&run_dir);
    println!(
        "EVIDENCE reverify resume_dry_run BEFORE: report={} outcome=needs_remediation ledger_subjects=TASK-X-010,TASK-X-020",
        finding_report["id"]
    );
    let dry_run = run(root, &["workflow", "resume", "--dry-run", &run_id]);
    assert!(
        dry_run.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&dry_run.stdout),
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_text = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        dry_text.contains(&format!(
            "reverify_latest_report: id={} outcome=needs_remediation",
            finding_report["id"].as_str().unwrap()
        )),
        "{dry_text}"
    );
    assert!(
        dry_text.contains("reauthor_subjects=TASK-X-010,TASK-X-020"),
        "{dry_text}"
    );
    assert_eq!(
        fs::read(run_dir.join("state.json")).unwrap(),
        state_before_dry_run
    );
    assert_eq!(fs::read(&latest_ledger).unwrap(), ledger_before_dry_run);
    assert_eq!(
        bodies
            .iter()
            .map(fs::read)
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        bodies_before_dry_run
    );
    assert_eq!(record_bytes(&run_dir), results_before_dry_run);
    assert_eq!(tree_bytes(&run_dir), run_bytes_before_dry_run);
    println!(
        "EVIDENCE reverify resume_dry_run AFTER: report={} outcome=needs_remediation exact_plan=true run_bytes_unchanged=true ledger_unchanged=true bodies_unchanged=true records_unchanged=true",
        finding_report["id"]
    );

    let ids_before_resume = archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2"))
        .load_call_records()
        .unwrap()
        .into_iter()
        .map(|record| record.call.id)
        .collect::<std::collections::BTreeSet<_>>();
    println!(
        "EVIDENCE reverify resume BEFORE: run=Paused report={} ledger_subjects=TASK-X-010,TASK-X-020 bodies_exist=true",
        finding_report["id"]
    );
    let resumed = run(root, &["workflow", "resume", "--live", "--yes", &run_id]);
    assert!(
        resumed.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&resumed.stdout),
        String::from_utf8_lossy(&resumed.stderr)
    );
    let resumed_records = archon_workflow::WorkflowV2ResultStore::new(run_dir.join("v2"))
        .load_call_records()
        .unwrap();
    let newly_authored = resumed_records
        .iter()
        .filter(|record| !ids_before_resume.contains(&record.call.id))
        .filter(|record| record.call.id.starts_with("body-"))
        .map(|record| record.call.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        newly_authored
            .iter()
            .filter(|id| id.contains("TASK-X-010"))
            .count(),
        1,
        "{newly_authored:?}"
    );
    assert_eq!(
        newly_authored
            .iter()
            .filter(|id| id.contains("TASK-X-020"))
            .count(),
        1,
        "{newly_authored:?}"
    );
    assert_eq!(
        newly_authored.len(),
        2,
        "unexpected body author calls: {newly_authored:?}"
    );
    let new_set_gates = resumed_records
        .iter()
        .filter(|record| !ids_before_resume.contains(&record.call.id))
        .filter_map(|record| {
            record
                .call
                .options
                .host_command
                .as_ref()
                .map(|request| request.command_id.as_str())
        })
        .filter(|id| *id == "task-set-lint" || *id == "requirements-trace")
        .collect::<Vec<_>>();
    assert!(
        new_set_gates.contains(&"task-set-lint"),
        "{new_set_gates:?}"
    );
    assert!(
        new_set_gates.contains(&"requirements-trace"),
        "{new_set_gates:?}"
    );
    let consumed =
        fs::read_to_string(run_dir.join("decomposition/reverify/consumed.jsonl")).unwrap();
    let consumed: serde_json::Value =
        serde_json::from_str(consumed.lines().last().unwrap()).unwrap();
    assert_eq!(consumed["report_id"], finding_report["id"]);
    let status = run(root, &["workflow", "status", &run_id]);
    assert!(status.status.success());
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(
        status_text.contains("latest_report: id=fixed-decomposition-final outcome=accepted"),
        "{status_text}"
    );
    assert!(
        status_text.contains(&format!(
            "superseded_id={}",
            finding_report["id"].as_str().unwrap()
        )),
        "{status_text}"
    );
    println!(
        "EVIDENCE reverify finding AFTER: outcome={} report={} supersedes={} verdicts=TASK-X-010:{},TASK-X-020:{} ledger_report_matches=true ledger_names_subjects=true run=Completed new_body_authors=exactly_two set_gates=both consumed_append_present latest_report=accepted resume_printed=true",
        finding_report["outcome"],
        finding_path.file_name().unwrap().to_string_lossy(),
        finding_report["supersedes_id"],
        subject["verdict"],
        second_subject["verdict"]
    );
}
