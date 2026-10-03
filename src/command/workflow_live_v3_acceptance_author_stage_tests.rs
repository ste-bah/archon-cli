//! REM-13 / ACC-A7 at the stage: a task set with no frozen contract gets one
//! authored and frozen, and a frozen contract that falls short of its PRD
//! gets its owed checks authored and added -- and either way the round runs
//! them. Every authored check is first proven able to fail on the tree
//! before implementation (M5).

use super::repair_tests::{Run, run_fixture_with, stage};
use super::*;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use crate::command::workflow_task_set::republish::test_fixture::{criterion, write_chain};
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, FreezeGateMode, TASK_SKELETON_FILE,
    TASK_SKELETON_LOCK_FILE, content_digest,
};

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The project as a checkout whose HEAD -- the tree before implementation
/// -- lacks `present`, which the live tree then has: a check that tests it
/// fails there and passes here.
fn pre_implementation_head(run: &Run) {
    let project = run.set.project.path();
    std::fs::write(project.join(".gitignore"), ".archon/\n").unwrap();
    let present = std::fs::read(project.join("present")).unwrap();
    std::fs::remove_file(project.join("present")).unwrap();
    git(project, &["init", "-q"]);
    git(
        project,
        &["config", "user.email", "fixture@example.invalid"],
    );
    git(project, &["config", "user.name", "fixture"]);
    git(project, &["add", "."]);
    git(project, &["commit", "-qm", "before implementation"]);
    std::fs::write(project.join("present"), present).unwrap();
}

/// Strip the frozen chain, leaving a task set the host must freeze, with
/// its decomposition's record of the PRD.
fn unfrozen(run: &Run) {
    for file in [
        ACCEPTANCE_CONTRACT_FILE,
        ACCEPTANCE_LOCK_FILE,
        TASK_SKELETON_FILE,
        TASK_SKELETON_LOCK_FILE,
    ] {
        std::fs::remove_file(run.set.tasks.join(file)).unwrap();
    }
    std::fs::remove_file(run.set.pin_path()).unwrap();
    let record = run
        .set
        .project
        .path()
        .join(".archon/workflows/decomposition-1/decomposition");
    std::fs::create_dir_all(&record).unwrap();
    let state = serde_json::json!({"identity": {
        "task_root_identity": run.set.tasks.display().to_string(),
        "prd_identity": run.set.prd.display().to_string(),
    }});
    std::fs::write(record.join("state.json"), state.to_string()).unwrap();
}

fn accepting() -> ScriptedAuthorJudge {
    ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f present && test -s present"),
        |_, _| true,
    )
}

/// REM-13: no contract, no lock, no pin. The host reads the PRD its
/// decomposition recorded, authors a check for each acceptance id, proves
/// each can fail before implementation, freezes the contract through the
/// enforce gate, and runs it in the round.
#[tokio::test]
async fn a_task_set_without_a_contract_gets_one_authored_frozen_and_run() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    unfrozen(&run);
    pre_implementation_head(&run);
    let client = accepting();
    let (result, record) = stage(&run, &client).await;
    assert!(record.contract_present, "{:?}", record.operational_errors);
    let repair = &record.contract_repairs[0];
    assert!(repair.repaired, "{repair:?}");
    assert_eq!(repair.trigger, author::REPAIR_TRIGGER_AUTHORED);
    assert_eq!(repair.check_ids, ["AC-F-001"]);
    assert!(run.set.tasks.join(ACCEPTANCE_LOCK_FILE).exists(), "frozen");
    assert!(run.set.pin_path().exists(), "pinned");
    assert_eq!(run.set.contract().prd.path, "prds/PRD-F.md");
    assert_eq!(record.passed_check_ids(), vec!["AC-F-001"]);
    assert!(
        record.operational_errors.is_empty(),
        "{:?}",
        record.operational_errors
    );
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.data["final"], true);
}

/// M5: with no run base commit and no checkout to fall back to, no
/// authored check can be proven able to fail: a round error naming why,
/// and nothing is frozen.
#[tokio::test]
async fn without_a_pre_implementation_tree_nothing_is_authored_or_frozen() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    unfrozen(&run);
    let client = accepting();
    let (_, record) = stage(&run, &client).await;
    let errors = record.operational_errors.join("\n");
    assert!(errors.contains("no pre-implementation tree"), "{errors}");
    assert!(
        !run.set.tasks.join(ACCEPTANCE_LOCK_FILE).exists(),
        "nothing frozen"
    );
    assert_eq!(client.authored(), 0, "nothing authored unproven");
}

/// REM-13: what could not be authored is owed by name, and what was is kept
/// for the next round, which authors only the rest (progress, not a cap).
#[tokio::test]
async fn an_unauthored_check_is_owed_by_name_and_the_next_round_authors_only_it() {
    let run = run_fixture_with(&[
        ("AC-F-001", "test -f present", true),
        ("AC-F-002", "test -f present", true),
    ]);
    unfrozen(&run);
    pre_implementation_head(&run);
    let refusing = ScriptedAuthorJudge::new(
        |entry, attempt| {
            command_entry(
                entry,
                &format!("test -f present && test -s present # {attempt}"),
            )
        },
        |id, _| id != "AC-F-002",
    );
    let (_, record) = stage(&run, &refusing).await;
    assert!(!record.contract_present);
    let errors = record.operational_errors.join("\n");
    assert!(
        errors.contains("acceptance check AC-F-002 is owed"),
        "{errors}"
    );
    assert!(
        !errors.contains("acceptance check AC-F-001 is owed"),
        "{errors}"
    );
    assert!(
        !run.set.tasks.join(ACCEPTANCE_LOCK_FILE).exists(),
        "nothing half-frozen"
    );
    let client = accepting();
    let (_, record) = stage(&run, &client).await;
    assert!(
        (client.judged_ids.lock().unwrap().iter()).all(|id| id == "AC-F-002"),
        "only the owed check is authored again"
    );
    assert!(
        record.contract_repairs.iter().any(|r| r.repaired),
        "{record:?}"
    );
    assert_eq!(record.passed_check_ids(), vec!["AC-F-001", "AC-F-002"]);
}

/// ACC-A7: a frozen contract with no `covers` owes each PRD requirement its
/// supplementary check. The host authors it with the freeze-time judge,
/// adds it to the frozen chain through the recorded republish (a lineage
/// link naming the PRD rebind), and the round runs it with every other check.
#[tokio::test]
async fn an_owed_supplementary_check_is_authored_added_to_the_chain_and_run() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    pre_implementation_head(&run);
    let before = run.set.contract();
    let prd = std::fs::read_to_string(&run.set.prd).unwrap();
    std::fs::write(
        &run.set.prd,
        format!("{prd}\n- REQ-F-001: the store keeps raw responses.\n"),
    )
    .unwrap();
    let (_, record) = stage(&run, &accepting()).await;
    assert!(
        record.operational_errors.is_empty(),
        "{:?}",
        record.operational_errors
    );
    let repair = (record.contract_repairs.iter())
        .find(|r| r.trigger == author::REPAIR_TRIGGER_AUTHORED)
        .expect("the extension is recorded");
    assert!(repair.repaired, "{repair:?}");
    assert_eq!(repair.check_ids, ["SUP-REQ-F-001"]);
    let after = run.set.contract();
    assert_eq!(
        after.acceptance, before.acceptance,
        "kept entries untouched"
    );
    assert_eq!(after.supplementary[0].covers, ["REQ-F-001"]);
    let link = run
        .set
        .pin()
        .lineage
        .last()
        .cloned()
        .expect("a lineage link");
    assert!(
        link.trigger.contains("rebound from PRD digest"),
        "{}",
        link.trigger
    );
    assert_eq!(record.passed_check_ids(), vec!["AC-F-001", "SUP-REQ-F-001"]);
}

/// M3: what the contract still owes its PRD after the round is ALWAYS held
/// against it: when the owed check cannot be authored, the round carries
/// both the per-check debt and the contract's drift.
#[tokio::test]
async fn drift_is_held_on_the_final_contract_even_while_it_is_being_authored() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    pre_implementation_head(&run);
    let prd = std::fs::read_to_string(&run.set.prd).unwrap();
    std::fs::write(
        &run.set.prd,
        format!("{prd}\n- REQ-F-001: the store keeps raw responses.\n"),
    )
    .unwrap();
    let refusing = ScriptedAuthorJudge::new(
        |entry, attempt| command_entry(entry, &format!("test -f present # {attempt}")),
        |_, _| false,
    );
    let (_, record) = stage(&run, &refusing).await;
    let errors = record.operational_errors.join("\n");
    assert!(
        errors.contains("acceptance check SUP-REQ-F-001 is owed"),
        "{errors}"
    );
    assert!(
        errors.contains("no frozen check covers PRD requirement"),
        "{errors}"
    );
}

/// M3: a supplementary check that does not cover the requirement it is
/// owed for is re-authored in place -- never dropped as "already held".
#[tokio::test]
async fn a_supplementary_check_that_covers_nothing_is_reauthored() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    pre_implementation_head(&run);
    let prd = format!(
        "{}\n- REQ-F-001: the store keeps raw responses.\n",
        std::fs::read_to_string(&run.set.prd).unwrap()
    );
    std::fs::write(&run.set.prd, &prd).unwrap();
    let mut contract = run.set.contract();
    contract.prd.digest = content_digest(prd.as_bytes());
    contract
        .supplementary
        .push(criterion("SUP-REQ-F-001", "test -f present", true));
    let project = run.set.project.path().to_path_buf();
    write_chain(
        &project,
        &run.set.tasks,
        &contract,
        FreezeGateMode::Observe,
        0,
    );
    let (_, record) = stage(&run, &accepting()).await;
    assert!(
        record.operational_errors.is_empty(),
        "{:?}",
        record.operational_errors
    );
    let after = run.set.contract();
    assert_eq!(after.supplementary.len(), 1, "re-authored in place");
    assert_eq!(after.supplementary[0].covers, ["REQ-F-001"]);
    assert_ne!(after.supplementary[0], contract.supplementary[0]);
}

/// M4: after the PRD moved, an entry whose criterion is no longer the PRD's
/// text is re-authored against the text as it is now.
#[tokio::test]
async fn an_entry_whose_criterion_the_prd_changed_is_reauthored() {
    let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
    pre_implementation_head(&run);
    let prd = std::fs::read_to_string(&run.set.prd).unwrap();
    std::fs::write(
        &run.set.prd,
        prd.replace("criterion text for AC-F-001", "the criterion as restated"),
    )
    .unwrap();
    let (_, record) = stage(&run, &accepting()).await;
    assert!(
        record.operational_errors.is_empty(),
        "{:?}",
        record.operational_errors
    );
    let after = run.set.contract();
    assert_eq!(after.acceptance[0].criterion, "the criterion as restated");
    assert!(
        (record.contract_repairs.iter()).any(|r| r.repaired && r.check_ids == ["AC-F-001"]),
        "{:?}",
        record.contract_repairs
    );
}

async fn resumed_unstamped_entry(extension: bool) {
    for stamp in [
        serde_json::json!(null),
        serde_json::json!({}),
        serde_json::json!({"schema": 0, "evidence_verdict": "accepted", "input_digest": "stale"}),
        serde_json::json!({"schema": 3, "evidence_verdict": "refuted", "input_digest": "stale"}),
        serde_json::json!({"schema": 3, "evidence_verdict": "accepted", "input_digest": "stale"}),
    ] {
        let run = run_fixture_with(&[("AC-F-001", "test -f present", true)]);
        pre_implementation_head(&run);
        let id = if extension {
            "SUP-REQ-F-001"
        } else {
            "AC-F-001"
        };
        if extension {
            let prd = std::fs::read_to_string(&run.set.prd).unwrap();
            std::fs::write(
                &run.set.prd,
                format!("{prd}\n- REQ-F-001: the store keeps raw responses.\n"),
            )
            .unwrap();
        } else {
            unfrozen(&run);
        }
        let prd = std::fs::read_to_string(&run.set.prd).unwrap();
        let mut entry = criterion(id, "echo 'refusing rule' >&2; exit 1", true);
        entry.criterion = if extension {
            "the store keeps raw responses.".into()
        } else {
            "criterion text for AC-F-001".into()
        };
        if extension {
            entry.covers = vec!["REQ-F-001".into()];
        }
        let path = run
            .store
            .run_dir(&run.run_id)
            .join("v2/acceptance-authoring.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::json!({"prd_digest": content_digest(prd.as_bytes()), "entries": {id: entry}, "passability": {id: stamp}, "feedback": {}}).to_string()).unwrap();
        let client = ScriptedAuthorJudge::new(
            |entry, _| command_entry(entry, "test -f present && test -s present"),
            |_, check| !check["command"].as_str().unwrap().contains("refusing rule"),
        );
        let (_, record) = stage(&run, &client).await;
        assert_eq!(
            client.authored(),
            1,
            "unstamped entry bypassed the evidence judge (extension={extension}): {record:?}"
        );
        assert!(client.prompts.lock().unwrap()[0].contains("cannot pass as written"));
        assert!(
            record.contract_repairs.iter().any(|repair| repair.repaired),
            "{record:?}"
        );
    }
}

#[tokio::test]
async fn r7_resumed_unstamped_entry_clears_evidence_before_fresh_publication() {
    resumed_unstamped_entry(false).await;
}

#[tokio::test]
async fn r7_resumed_unstamped_entry_clears_evidence_before_extension_publication() {
    resumed_unstamped_entry(true).await;
}
