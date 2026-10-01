//! Batch O: the host's remediation plan routes every finding, at any
//! severity, and grants only from its own reading.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use serde_json::json;

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "crates/x/src/lib.rs",
        "crates/x/src/extra.rs",
        "crates/y/src/lib.rs",
        "crates/z/src/orphan.rs",
        "crates/z/tests/orphan_test.rs",
    ] {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    std::fs::create_dir_all(dir.path().join("tasks")).unwrap();
    std::fs::write(
        dir.path().join("tasks/T-A.md"),
        "Owns crates/x/src/lib.rs\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("tasks/T-B.md"),
        "Owns crates/y/src/lib.rs; related: crates/z/tests/orphan_test.rs\n",
    )
    .unwrap();
    dir
}

fn task(id: &str, owns: &[&str], implements: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        implements: implements.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task(
                "T-A",
                &["crates/x/src/lib.rs", "crates/x/src/extra.rs"],
                &["REQ-1"],
            ),
            task("T-B", &["crates/y/src/lib.rs"], &["REQ-2"]),
        ],
    }
}

fn entry(plan: &Value, index: usize) -> &Value {
    &plan["findings"][index]
}

#[test]
fn a_finding_naming_no_task_is_routed_by_content_at_any_severity() {
    let dir = repo();
    let findings = vec![
        // Names a file T-B owns: T-B fixes it, though it is "low".
        json!({"severity": "low", "claim": "crates/y/src/lib.rs drops the error"}),
        // Names an unowned file only T-B's text names: T-B, granted.
        json!({"severity": "info", "claim": "crates/z/tests/orphan_test.rs is owned by no task"}),
        // Cites a requirement T-A implements.
        json!({"severity": "nit", "claim": "REQ-1 is never exercised"}),
        // Names nothing at all: every task that may write, one cross unit.
        json!({"severity": "low", "claim": "the reviewers could not run commands"}),
    ];
    let plan = plan(&findings, Some(&universe()), Some(dir.path()));
    assert_eq!(plan["placed"], json!(true));
    assert_eq!(entry(&plan, 0)["task_ids"], json!(["T-B"]));
    assert_eq!(entry(&plan, 1)["task_ids"], json!(["T-B"]));
    assert_eq!(
        entry(&plan, 1)["grants"],
        json!(["crates/z/tests/orphan_test.rs"])
    );
    assert_eq!(entry(&plan, 2)["task_ids"], json!(["T-A"]));
    assert_eq!(entry(&plan, 3)["task_ids"], json!(["T-A", "T-B"]));
    assert_eq!(entry(&plan, 3)["cross"], json!(true));
    for (index, finding) in findings.iter().enumerate().take(4) {
        assert_eq!(
            entry(&plan, index)["finding_id"],
            json!(finding_id_of(finding))
        );
    }
    // Every planned task's declared files are its scope, whatever the
    // script listed.
    assert_eq!(
        plan["task_scope"]["T-A"],
        json!(["crates/x/src/extra.rs", "crates/x/src/lib.rs"])
    );
}

#[test]
fn a_finding_naming_another_tasks_file_spans_both_and_grants_never_come_from_its_fields() {
    let dir = repo();
    let findings = vec![json!({
        "canonical_task_ids": ["T-A"],
        "claim": "the fix lives in crates/y/src/lib.rs and crates/z/src/orphan.rs",
        // An agent's own grant list is never read.
        "grants": ["crates/x/src/secret.rs"],
    })];
    let plan = plan(&findings, Some(&universe()), Some(dir.path()));
    assert_eq!(entry(&plan, 0)["task_ids"], json!(["T-A", "T-B"]));
    assert_eq!(entry(&plan, 0)["cross"], json!(true));
    assert_eq!(entry(&plan, 0)["grants"], json!(["crates/z/src/orphan.rs"]));
}

#[test]
fn without_a_universe_nothing_is_read_as_placed() {
    let findings = vec![json!({"claim": "x"})];
    let plan = plan(&findings, None, None);
    assert_eq!(plan["placed"], json!(false));
    assert_eq!(entry(&plan, 0)["task_ids"], json!([]));
}

#[test]
fn a_check_finding_is_flagged() {
    let dir = repo();
    let findings = vec![
        json!({"canonical_task_ids": ["T-A"], "claim": "The artifact test only checks that validation.json exists, never its status"}),
        json!({"canonical_task_ids": ["T-A"], "claim": "The gap audit doc miscounts rows"}),
    ];
    let plan = plan(&findings, Some(&universe()), Some(dir.path()));
    assert_eq!(entry(&plan, 0)["check"], json!(true));
    assert_eq!(entry(&plan, 1)["check"], json!(false));
}

#[test]
fn a_brace_list_names_each_file_it_spells_out() {
    let dir = repo();
    let named = explicitly_named(
        "the fork `crates/x/src/{lib.rs,extra.rs,gone.rs}` diverged",
        dir.path(),
    );
    assert_eq!(named, ["crates/x/src/extra.rs", "crates/x/src/lib.rs"]);
    // No brace list: nothing more than the literal names.
    assert!(explicitly_named("crates/x/src/{lib.rs}", dir.path()).is_empty());
}
