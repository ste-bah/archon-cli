use super::*;
use crate::command::workflow_task_set::reauthor::AuthorScope;
use crate::command::workflow_task_set::reauthor::test_client::{
    ScriptedAuthorJudge, command_entry,
};
use crate::command::workflow_task_set::republish::test_fixture::frozen_set_proven as frozen_set;
use crate::command::workflow_task_set::republish::{ReauthorRequest, reauthor_and_republish};

#[tokio::test]
async fn a_republish_pins_the_chain_it_publishes_and_keeps_every_other_checks_pins() {
    let set = frozen_set(&[
        ("AC-F-001", "bash scripts/one.sh", true),
        ("AC-F-002", "bash scripts/two.sh", false),
    ]);
    let scripts = set.project.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("one.sh"), "exit 1\n").unwrap();
    std::fs::write(scripts.join("two.sh"), "exit 1\n").unwrap();
    // A whole-set freeze pins every check fresh.
    let (path, bytes) =
        frozen_sidecar(set.project.path(), &set.tasks, &set.contract_bytes(), None).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let store = PinStore::frozen(set.project.path(), &set.tasks);
    let frozen = store.read().unwrap().unwrap();
    assert_eq!(frozen.origin, ORIGIN_FREEZE);
    assert_eq!(
        frozen.acceptance_digest,
        content_digest(&set.contract_bytes())
    );
    // Drift on a check nobody re-authors: the republish must not bless it.
    std::fs::write(scripts.join("one.sh"), "exit 0\n").unwrap();
    std::fs::write(scripts.join("three.sh"), "exit 1\n").unwrap();
    let named: BTreeSet<String> = ["AC-F-002".to_string()].into();
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "bash scripts/three.sh"),
        |_, _| true,
    );
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd);
    reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: set.gate(),
            trigger: "test",
        },
        &scope,
    )
    .await
    .expect("repair publishes");
    let republished = store.read().unwrap().unwrap();
    assert_eq!(republished.origin, ORIGIN_REPUBLISH);
    assert_eq!(
        republished.acceptance_digest,
        content_digest(&set.contract_bytes()),
        "the sidecar binds the contract it was published with"
    );
    assert_eq!(republished.checks["AC-F-001"], frozen.checks["AC-F-001"]);
    assert_eq!(
        republished.checks["AC-F-002"].sources[0].path,
        "scripts/three.sh"
    );
}

fn republish_two(
    set: &crate::command::workflow_task_set::republish::test_fixture::FrozenSet,
) -> Result<()> {
    let named: BTreeSet<String> = ["AC-F-002".to_string()].into();
    let client = ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "bash scripts/three.sh"),
        |_, _| true,
    );
    let scope = AuthorScope::for_task_set(set.project.path(), &set.tasks, &set.prd);
    futures_block(reauthor_and_republish(
        &client,
        ReauthorRequest {
            project_root: set.project.path(),
            tasks_root: &set.tasks,
            prd_path: &set.prd,
            ids: &named,
            gate: set.gate(),
            trigger: "test",
        },
        &scope,
    ))
    .map(|_| ())
}

fn futures_block<F: std::future::Future>(future: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(future))
}

fn pre_plan11_set() -> crate::command::workflow_task_set::republish::test_fixture::FrozenSet {
    let set = frozen_set(&[
        ("AC-F-001", "bash scripts/one.sh", true),
        ("AC-F-002", "bash scripts/two.sh", false),
    ]);
    let scripts = set.project.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    for name in ["one.sh", "two.sh", "three.sh"] {
        std::fs::write(scripts.join(name), "exit 1\n").unwrap();
    }
    set
}

/// Item 6: a task set frozen before PLAN-11 has no sidecar; its republish
/// re-binds the pins the run recorded (a drifted source is not blessed).
#[tokio::test(flavor = "multi_thread")]
async fn a_pre_plan11_republish_rebinds_from_the_runs_pins() {
    let set = pre_plan11_set();
    let project = set.project.path();
    let run = project.join(".archon/workflows/run-1");
    let roots = Roots {
        repository: project,
        project,
    };
    let (_, recorded) =
        archon_workflow::check_source_pins::load_for_run(&run, project, &set.tasks, &roots)
            .unwrap()
            .unwrap();
    std::fs::write(project.join("scripts/one.sh"), "exit 0\n").unwrap();
    republish_two(&set).expect("republish");
    let sidecar = PinStore::frozen(project, &set.tasks)
        .read()
        .unwrap()
        .unwrap();
    assert_eq!(
        sidecar.checks["AC-F-001"], recorded.checks["AC-F-001"],
        "carried from the run"
    );
    assert_eq!(
        sidecar.checks["AC-F-002"].sources[0].path,
        "scripts/three.sh"
    );
}

/// Item 6: a republish of a check a run still holds a pending change for
/// refuses -- the change is settled first; and an unreadable sidecar fails
/// the republish instead of being pinned over.
#[tokio::test(flavor = "multi_thread")]
async fn a_republish_refuses_a_pending_change_and_an_unreadable_sidecar() {
    let set = pre_plan11_set();
    let project = set.project.path();
    let run = project.join(".archon/workflows/run-1");
    std::fs::create_dir_all(run.join("v2")).unwrap();
    archon_workflow::check_source_requests::record(
        &run,
        archon_workflow::check_source_requests::NewRequest {
            origin: archon_workflow::check_source_requests::ORIGIN_ACCEPTANCE_DRIFT,
            check_ids: ["AC-F-002".to_string()].into(),
            root: archon_workflow::check_source_resolve::SourceRoot::Project,
            path: "scripts/two.sh",
            item: None,
            was_pinned: true,
            pinned_digest: None,
            proposed: Some(b"exit 0\n"),
            proposed_file: None,
            landed_file_digest: None,
            call_id: "",
            branch_id: "",
            task_ids: Vec::new(),
        },
    )
    .unwrap();
    let error = republish_two(&set).unwrap_err();
    assert!(
        format!("{error:#}").contains("pending source change"),
        "{error:#}"
    );
    std::fs::remove_dir_all(run.join("v2/check-source-requests")).unwrap();
    let store = PinStore::frozen(project, &set.tasks);
    std::fs::create_dir_all(store.sidecar.parent().unwrap()).unwrap();
    std::fs::write(&store.sidecar, "not json").unwrap();
    let error = republish_two(&set).unwrap_err();
    assert!(format!("{error:#}").contains("malformed"), "{error:#}");
}
