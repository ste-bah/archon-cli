//! Repair safety: recorded gate modes, races, locks, rollback, reuse.

use super::super::test_fixture::{FIXTURE_JUDGE_MODEL, FrozenSet, frozen_set, frozen_set_in};
use super::super::*;
use super::ids;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};

fn request<'a>(set: &'a FrozenSet, ids: &'a BTreeSet<String>) -> ReauthorRequest<'a> {
    ReauthorRequest {
        project_root: set.project.path(),
        tasks_root: &set.tasks,
        prd_path: &set.prd,
        ids,
        gate: set.gate(),
        trigger: "test",
    }
}

fn scope(set: &FrozenSet) -> AuthorScope {
    AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd)
}

fn accept_all(command: &'static str) -> ScriptedAuthorJudge {
    ScriptedAuthorJudge::new(move |entry, _| command_entry(entry, command), |_, _| true)
}

#[tokio::test]
async fn an_enforce_frozen_chain_republishes_under_enforce() {
    let set = frozen_set_in(
        &[("AC-F-001", "jq -e '.a == true' out.json", true)],
        FreezeGateMode::Enforce,
        "",
    );
    let named = ids(&["AC-F-001"]);
    let client = accept_all("jq -e '.a == true and .b == 1' out.json");
    reauthor_and_republish(&client, request(&set, &named), &scope(&set))
        .await
        .expect("a clean repair publishes under enforce");
    let pin = set.pin();
    assert_eq!(pin.acceptance_gate.mode, FreezeGateMode::Enforce);
    assert_eq!(pin.skeleton_gate.unwrap().mode, FreezeGateMode::Enforce);
    validate_full_chain(&set.tasks, &set.pin()).unwrap();
}

#[tokio::test]
async fn an_enforce_gate_finding_refuses_the_repair_names_the_mode_and_writes_nothing() {
    // A duplicated obligation id is a PRD finding enforcement cannot publish.
    let set = frozen_set_in(
        &[("AC-F-001", "jq -e '.a == true' out.json", true)],
        FreezeGateMode::Enforce,
        "| REQ-F-001 | one |\n| REQ-F-001 | two |\n",
    );
    let before = set.chain_bytes();
    let named = ids(&["AC-F-001"]);
    let client = accept_all("jq -e '.a == true and .b == 1' out.json");
    let error = format!(
        "{:#}",
        reauthor_and_republish(&client, request(&set, &named), &scope(&set))
            .await
            .expect_err("enforce blocks the finding")
    );
    assert!(error.contains("under Enforce mode"), "{error}");
    assert_eq!(set.chain_bytes(), before);
}

#[tokio::test]
async fn a_chain_changed_while_the_author_ran_is_never_overwritten() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let skeleton_lock = set.tasks.join(TASK_SKELETON_LOCK_FILE);
    let racing = skeleton_lock.clone();
    let client = ScriptedAuthorJudge::new(
        move |entry, _| {
            // Another writer touches the chain mid-repair.
            let mut bytes = std::fs::read(&racing).unwrap();
            bytes.push(b'\n');
            std::fs::write(&racing, bytes).unwrap();
            command_entry(entry, "jq -e '.a == 1' out.json")
        },
        |_, _| true,
    );
    let before = std::fs::read(set.tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    let named = ids(&["AC-F-001"]);
    let error = reauthor_and_republish(&client, request(&set, &named), &scope(&set))
        .await
        .expect_err("changed underneath")
        .to_string();
    assert!(error.contains("changed since it was verified"), "{error}");
    assert_eq!(
        std::fs::read(set.tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
        before
    );
}

#[tokio::test]
async fn a_second_concurrent_repair_is_refused() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let _held = ChainLock::acquire(&set.pin_path(), &set.tasks).unwrap();
    let named = ids(&["AC-F-001"]);
    let client = accept_all("jq -e '.a == 1' out.json");
    let error = reauthor_and_republish(&client, request(&set, &named), &scope(&set))
        .await
        .expect_err("locked")
        .to_string();
    assert!(error.contains("another freeze or repair"), "{error}");
    assert_eq!(client.authored(), 0);
}

#[tokio::test]
async fn a_non_canonical_skeleton_is_refused_rather_than_rewritten() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let path = set.tasks.join(TASK_SKELETON_FILE);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["note"] = serde_json::json!("a field the host type would drop");
    let bytes = serde_json::to_vec_pretty(&value).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    // Keep the chain itself valid so only the canonical check can refuse.
    let digest = content_digest(&bytes);
    let lock_path = set.tasks.join(TASK_SKELETON_LOCK_FILE);
    let mut lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.digest = digest.clone();
    std::fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    let mut pin = set.pin();
    pin.skeleton_digest = Some(digest);
    std::fs::write(set.pin_path(), serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    let before = set.chain_bytes();
    let named = ids(&["AC-F-001"]);
    let error = reauthor_and_republish(&client_for_skeleton(), request(&set, &named), &scope(&set))
        .await
        .expect_err("non-canonical skeleton")
        .to_string();
    assert!(error.contains("canonical serialization"), "{error}");
    assert_eq!(set.chain_bytes(), before);
}

fn client_for_skeleton() -> ScriptedAuthorJudge {
    accept_all("jq -e '.a == 1' out.json")
}

#[test]
fn a_rolled_back_publish_restores_every_prior_file_and_leaves_no_debris() {
    let temp = tempfile::tempdir().unwrap();
    let existing = temp.path().join("existing.json");
    let created = temp.path().join("created.json");
    std::fs::write(&existing, b"old").unwrap();
    let anchor = tempfile::tempdir().unwrap();
    let transaction = begin_publish(
        &anchor.path().join("pin.json"),
        &[
            (existing.clone(), b"new".to_vec()),
            (created.clone(), b"fresh".to_vec()),
        ],
        "test",
        &[],
    )
    .unwrap();
    assert_eq!(std::fs::read(&existing).unwrap(), b"new");
    assert_eq!(std::fs::read(&created).unwrap(), b"fresh");
    transaction.roll_back().unwrap();
    assert_eq!(std::fs::read(&existing).unwrap(), b"old");
    assert!(!created.exists());
    let names = std::fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["existing.json".to_string()]);
}

#[test]
fn a_decomposition_never_builds_on_a_frozen_contract_with_a_refuted_check() {
    let set = frozen_set(&[
        ("AC-F-001", "jq -e '.a == true' out.json", true),
        ("AC-F-002", "jq -e '.b == true' out.json", false),
    ]);
    let error = crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
        set.project.path(),
        &set.prd,
        &set.tasks,
    )
    .expect_err("a refuted frozen contract is not reusable")
    .to_string();
    assert!(error.contains("--reauthor AC-F-002"), "{error}");
    assert!(
        error.contains(&format!("cd {} && ", set.project.path().display())),
        "{error}"
    );
}

#[test]
fn launch_refuses_a_frozen_contract_whose_pin_cannot_be_read() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", true)]);
    std::fs::remove_file(set.pin_path()).unwrap();
    let error = refuse_unaccepted_launch(set.project.path(), &set.tasks)
        .expect_err("unreadable pin")
        .to_string();
    assert!(error.contains("pin"), "{error}");
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", true)]);
    std::fs::write(set.tasks.join(ACCEPTANCE_CONTRACT_FILE), b"{ torn").unwrap();
    let error = refuse_unaccepted_launch(set.project.path(), &set.tasks)
        .expect_err("unreadable contract")
        .to_string();
    assert!(error.contains("cannot be read"), "{error}");
}

#[tokio::test]
async fn the_repair_is_judged_by_the_recorded_freeze_time_model() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", false)]);
    let named = ids(&["AC-F-001"]);
    let client = accept_all("jq -e '.a == 1' out.json");
    reauthor_and_republish(&client, request(&set, &named), &scope(&set))
        .await
        .unwrap();
    assert_eq!(
        *client.judged_models.lock().unwrap(),
        vec![FIXTURE_JUDGE_MODEL.to_string()]
    );
    let sampling = set.contract().acceptance[0]
        .judgment
        .sampling
        .clone()
        .unwrap();
    assert_eq!(sampling["model"], FIXTURE_JUDGE_MODEL);
}

fn rewrite_sampling(set: &FrozenSet, edit: impl Fn(usize, &mut serde_json::Value)) {
    let mut contract = set.contract();
    for (index, entry) in contract.acceptance.iter_mut().enumerate() {
        let mut sampling = entry.judgment.sampling.clone().unwrap_or_default();
        edit(index, &mut sampling);
        entry.judgment.sampling = (!sampling.is_null()).then_some(sampling);
    }
    super::super::test_fixture::write_chain(
        set.project.path(),
        &set.tasks,
        &contract,
        FreezeGateMode::Observe,
        1,
    );
}

/// A named edit to a contract's JSON, given the judge entry's index.
type ContractEdit = Box<dyn Fn(usize, &mut serde_json::Value)>;

#[tokio::test]
async fn a_contract_without_one_recorded_judge_or_on_another_provider_is_refused() {
    let cases: Vec<(&str, ContractEdit)> = vec![
        (
            "records no judge",
            Box::new(|_, s| *s = serde_json::Value::Null),
        ),
        (
            "more than one judge",
            Box::new(|i, s| s["model"] = serde_json::json!(format!("judge-{i}"))),
        ),
        (
            "use that provider",
            Box::new(|_, s| s["provider"] = serde_json::json!("elsewhere")),
        ),
    ];
    for (expected, edit) in cases {
        let set = frozen_set(&[
            ("AC-F-001", "jq -e '.a == true' out.json", true),
            ("AC-F-002", "jq -e '.b == true' out.json", false),
        ]);
        rewrite_sampling(&set, edit);
        let before = set.chain_bytes();
        let named = ids(&["AC-F-002"]);
        let client = accept_all("jq -e '.b == 1' out.json");
        let error = reauthor_and_republish(&client, request(&set, &named), &scope(&set))
            .await
            .expect_err(expected)
            .to_string();
        assert!(error.contains(expected), "{expected}: {error}");
        assert_eq!(client.authored(), 0, "{expected}: no author call");
        assert_eq!(set.chain_bytes(), before);
    }
}

#[test]
fn the_whole_set_publishers_take_the_chain_lock() {
    let set = frozen_set(&[("AC-F-001", "jq -e '.a == true' out.json", true)]);
    let _held = ChainLock::acquire(&set.pin_path(), &set.tasks).unwrap();
    let pin = set.pin();
    let lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(set.tasks.join(ACCEPTANCE_LOCK_FILE)).unwrap())
            .unwrap();
    let error = publish::publish_acceptance_files(
        &set.tasks,
        set.project.path(),
        &set.contract_bytes(),
        &lock,
        &pin,
    )
    .expect_err("acceptance publisher waits for the lock")
    .to_string();
    assert!(error.contains("another freeze or repair"), "{error}");
    let skeleton_lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(set.tasks.join(TASK_SKELETON_LOCK_FILE)).unwrap())
            .unwrap();
    let error = publish::publish_skeleton_files(
        &set.tasks,
        &set.pin_path(),
        &std::fs::read(set.tasks.join(TASK_SKELETON_FILE)).unwrap(),
        &skeleton_lock,
        &pin,
    )
    .expect_err("skeleton publisher waits for the lock")
    .to_string();
    assert!(error.contains("another freeze or repair"), "{error}");
}

#[test]
fn a_target_that_moved_since_verification_is_never_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("chain.json");
    std::fs::write(&target, b"someone else's").unwrap();
    let anchor = tempfile::tempdir().unwrap();
    let error = begin_publish(
        &anchor.path().join("pin.json"),
        &[(target.clone(), b"ours".to_vec())],
        "test",
        &[(target.clone(), Some(content_digest(b"what we verified")))],
    )
    .err()
    .expect("moved target")
    .to_string();
    assert!(error.contains("changed since it was verified"), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"someone else's");
    assert_eq!(
        std::fs::read_dir(temp.path()).unwrap().count(),
        1,
        "no temps left"
    );
}

#[test]
fn rollback_never_restores_over_a_target_another_writer_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("chain.json");
    std::fs::write(&target, b"old").unwrap();
    let anchor = tempfile::tempdir().unwrap();
    let transaction = begin_publish(
        &anchor.path().join("pin.json"),
        &[(target.clone(), b"ours".to_vec())],
        "test",
        &[],
    )
    .unwrap();
    std::fs::write(&target, b"theirs").unwrap();
    let error = transaction
        .roll_back()
        .expect_err("not ours to undo")
        .to_string();
    assert!(error.contains("changed after this publish"), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"theirs");
}
