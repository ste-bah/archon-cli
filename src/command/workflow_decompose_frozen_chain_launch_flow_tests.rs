//! Whole launches on a frozen contract: the decision, the repair after the
//! task-root claim, and what each leaves on disk.
use super::*;
use sha2::{Digest, Sha256};

/// Stops a launch at provider construction: everything before it ran.
struct LaunchBarrier;

#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for LaunchBarrier {
    async fn build_client(
        &self,
        _request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>
    {
        Err(archon_workflow::WorkflowError::port("barrier observed"))
    }
}

async fn launch(fixture: &Fixture) -> anyhow::Error {
    launch_with(fixture, &LaunchBarrier).await
}

async fn launch_with(
    fixture: &Fixture,
    factory: &dyn archon_workflow::WorkflowLlmClientFactory,
) -> anyhow::Error {
    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.repository_root = Some(fixture.project.clone());
    crate::command::workflow_decompose::run_fixed_decomposition_with_factory(
        &fixture.project,
        Path::new("prds/PRD-X.md"),
        Path::new("tasks/PRD-X"),
        None,
        true,
        &config,
        &archon_core::env_vars::load_env_vars_from(&std::collections::HashMap::new()),
        factory,
    )
    .await
    .unwrap_err()
}

/// The re-author's client first (scripted author and judge), then the
/// barrier for the run's own client.
struct ReauthorThenBarrier {
    reauthor: std::sync::Mutex<Option<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>>,
    origins: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for ReauthorThenBarrier {
    async fn build_client(
        &self,
        request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>
    {
        self.origins.lock().unwrap().push(request.origin);
        match self.reauthor.lock().unwrap().take() {
            Some(client) => Ok(client),
            None => Err(archon_workflow::WorkflowError::port("barrier observed")),
        }
    }
}

#[tokio::test]
async fn a_launch_re_authors_a_frozen_check_this_builds_rules_refuse_and_goes_on() {
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };
    let fixture = frozen(Some(names_live_root));
    // The pre-implementation tree the repaired check is proven on.
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);
    let client = std::sync::Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f marker"),
        |_, _| true,
    ));
    let factory = ReauthorThenBarrier {
        reauthor: std::sync::Mutex::new(Some(client.clone())),
        origins: std::sync::Mutex::new(Vec::new()),
    };
    let error = format!("{:#}", launch_with(&fixture, &factory).await);
    assert!(
        error.contains("barrier observed"),
        "the launch went on: {error}"
    );
    assert_eq!(
        factory.origins.lock().unwrap()[0],
        "workflow-decompose-launch-reauthor"
    );
    assert_eq!(client.authored(), 1, "only the refused check is authored");
    // The judge asks once per sample; it is asked of the refused check only.
    let judged = client.judged_ids.lock().unwrap().clone();
    assert!(
        !judged.is_empty() && judged.iter().all(|id| id == "AC-X-001"),
        "{judged:?}"
    );
    let prompts = client.prompts.lock().unwrap().join("\n");
    assert!(
        prompts.contains("acceptance entry 'AC-X-001' was refused"),
        "the author is shown the rule that refused the check: {prompts}"
    );
    let contract: AcceptanceContract = serde_json::from_slice(
        &std::fs::read(fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap(),
    )
    .unwrap();
    assert!(
        matches!(
            &contract.acceptance[0].check,
            archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. }
                if command == "test -f marker"
        ),
        "{:?}",
        contract.acceptance[0].check
    );
    let log = std::fs::read_to_string(fixture.tasks.join(".decompose.log")).unwrap();
    assert!(
        log.contains(
            "frozen_chain_launch acceptance=reauthor ids=AC-X-001 kept=0 reason=unrecorded"
        ),
        "{log}"
    );
    assert!(
        log.contains("frozen_chain_launch acceptance=reauthored ids=AC-X-001 acceptance_digest="),
        "{log}"
    );
    let run = fixture.store.list_runs().unwrap().pop().unwrap();
    let events = std::fs::read_to_string(fixture.store.events_path(&run.id)).unwrap();
    let event: serde_json::Value = events
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| {
            event["detail"]["event"]
                == crate::command::workflow_decompose_launch_repair::REAUTHORED_EVENT
        })
        .unwrap_or_else(|| panic!("no repair event: {events}"));
    assert_eq!(event["detail"]["ids"], serde_json::json!(["AC-X-001"]));
    assert!(
        event["detail"]["refusals"]["AC-X-001"]
            .as_str()
            .unwrap()
            .contains("was refused")
    );
    // The run builds on the republished contract, verified in place.
    let arguments: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            fixture
                .store
                .run_dir(&run.id)
                .join(crate::command::workflow_decompose::FIXED_ARGUMENTS_PATH),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(arguments["frozenChain"]["acceptance"], true, "{arguments}");
    let pin: AcceptancePinView = serde_json::from_slice(
        &std::fs::read(crate::command::workflow_task_set::acceptance_pin_path(
            &fixture.project,
            &fixture.tasks,
        ))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        pin.acceptance_digest,
        content_digest(&std::fs::read(fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap())
    );
}

#[derive(serde::Deserialize)]
struct AcceptancePinView {
    acceptance_digest: String,
}

#[tokio::test]
async fn a_launch_on_a_frozen_contract_logs_its_decision() {
    let fixture = frozen(None);
    let error = format!("{:#}", launch(&fixture).await);
    assert!(error.contains("barrier observed"), "{error}");
    let log = std::fs::read_to_string(fixture.tasks.join(".decompose.log")).unwrap();
    assert!(
        log.contains(
            "frozen_chain_launch acceptance=verify reason=unrecorded recorded freeze logic"
        ),
        "{log}"
    );
    let run = fixture.store.list_runs().unwrap().pop().unwrap();
    let inventory = fixture
        .store
        .run_dir(&run.id)
        .join(crate::command::workflow_decompose_requirement_inventory::INVENTORY_PATH);
    let inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(inventory).unwrap()).unwrap();
    assert_eq!(
        inventory["ids"],
        serde_json::json!(["REQ-X-001"]),
        "{inventory}"
    );
    assert_eq!(inventory["recorded_at"], "launch");
}

/// Every file of the frozen chain under the fixture's task root, with a
/// two-task skeleton frozen on the current contract.
fn with_skeleton(fixture: &Fixture) -> Vec<PathBuf> {
    let contract = std::fs::read(fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    crate::command::workflow_decompose_frozen_chain::test_support::freeze_skeleton(
        &fixture.project,
        &fixture.tasks,
        &content_digest(&contract),
        &["TASK-X-010"],
    );
    vec![
        fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE),
        fixture.tasks.join(ACCEPTANCE_LOCK_FILE),
        fixture
            .tasks
            .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE),
        fixture
            .tasks
            .join(archon_workflow::task_set_contract::TASK_SKELETON_LOCK_FILE),
        crate::command::workflow_task_set::acceptance_pin_path(&fixture.project, &fixture.tasks),
    ]
}

fn read_all(paths: &[PathBuf]) -> Vec<Vec<u8>> {
    paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect()
}
fn tree_fingerprint(root: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(&path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                pending.push(path);
            } else if meta.is_file() {
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                let digest = format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap()));
                files.push((relative, digest));
            }
        }
    }
    files.sort();
    files
}

/// A paused fixed decomposition that owns the fixture's task root.
fn paused_owner(fixture: &Fixture) -> String {
    let run = fixture
        .store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "owner".into(),
            task: "paused decomposition".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let text = |path: &Path| path.to_string_lossy().replace('\\', "/");
    let state = archon_workflow::FixedDecompositionStateV1 {
        schema_version: archon_workflow::FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION,
        run_kind: archon_workflow::WorkflowRunKind::FixedDecompositionV1,
        identity: archon_workflow::FixedRunIdentityV1 {
            template_version: "fixed-decomposition-v1".into(),
            starting_binary_revision: "rev".into(),
            script_digest: "script".into(),
            catalog_digest: "catalog".into(),
            project_root_identity: text(&fixture.project),
            prd_identity: text(&fixture.project.join("prds/PRD-X.md")),
            task_root_identity: text(&fixture.tasks),
        },
        phase: archon_workflow::DecompositionPhase::Acceptance,
        attempts: Default::default(),
        dispositions: Default::default(),
        log_path: text(&fixture.tasks.join(".decompose.log")),
    };
    fixture
        .store
        .write_run_json(
            &run.id,
            crate::command::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH,
            &state,
        )
        .unwrap();
    let mut paused = fixture.store.load_state(&run.id).unwrap();
    paused.status = archon_workflow::RunStatus::Paused;
    fixture.store.save_state(&paused).unwrap();
    run.id
}

fn scripted_factory(
    accepts: bool,
) -> (
    ReauthorThenBarrier,
    std::sync::Arc<crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge>,
) {
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };
    let client = std::sync::Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f marker"),
        move |_, _| accepts,
    ));
    let factory = ReauthorThenBarrier {
        reauthor: std::sync::Mutex::new(Some(client.clone())),
        origins: std::sync::Mutex::new(Vec::new()),
    };
    (factory, client)
}

#[tokio::test]
async fn a_launch_on_a_task_root_a_paused_run_owns_re_authors_nothing() {
    let fixture = frozen(Some(names_live_root));
    with_skeleton(&fixture);
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);
    let owner = paused_owner(&fixture);
    let before = (
        tree_fingerprint(&fixture.tasks),
        tree_fingerprint(&fixture.store.run_dir(&owner)),
    );
    let (factory, client) = scripted_factory(true);
    let error = format!("{:#}", launch_with(&fixture, &factory).await);
    assert!(
        error.contains(&format!(
            "active fixed decomposition {owner} already owns task root"
        )),
        "{error}"
    );
    assert_eq!(
        (
            tree_fingerprint(&fixture.tasks),
            tree_fingerprint(&fixture.store.run_dir(&owner))
        ),
        before,
        "the owner's frozen chain was rewritten"
    );
    assert_eq!(client.authored(), 0, "no check was re-authored");
    assert!(
        factory.origins.lock().unwrap().is_empty(),
        "no client was built"
    );
    assert_eq!(fixture.store.list_runs().unwrap().len(), 1, "no second run");
}

#[tokio::test]
async fn a_paused_owner_with_pending_publication_keeps_the_entire_root_unchanged() {
    let fixture = frozen(Some(names_live_root));
    with_skeleton(&fixture);
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);
    let target = fixture.tasks.join("TASK-X-010.md");
    std::fs::write(&target, "before journal recovery\n").unwrap();
    let pin =
        crate::command::workflow_task_set::acceptance_pin_path(&fixture.project, &fixture.tasks);
    let _cleanup = crate::command::workflow_task_set::crash_publish(
        &pin,
        &[(target, b"after journal recovery\n".to_vec())],
        "committed",
        false,
    );
    let owner = paused_owner(&fixture);
    let before = (
        tree_fingerprint(&fixture.tasks),
        tree_fingerprint(&fixture.store.run_dir(&owner)),
    );
    let (factory, client) = scripted_factory(true);
    let error = format!("{:#}", launch_with(&fixture, &factory).await);
    assert!(
        error.contains(&format!(
            "active fixed decomposition {owner} already owns task root"
        )),
        "{error}"
    );
    assert_eq!(
        (
            tree_fingerprint(&fixture.tasks),
            tree_fingerprint(&fixture.store.run_dir(&owner))
        ),
        before,
        "pending publication recovery changed the owner's files before claim refusal"
    );
    assert_eq!(client.authored(), 0);
    assert!(factory.origins.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refuted_launch_repair_is_logged_and_leaves_the_chain_for_the_run_repair() {
    let fixture = frozen(Some(names_live_root));
    let chain = with_skeleton(&fixture);
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);
    let before = read_all(&chain);
    let (factory, client) = scripted_factory(false);
    let error = format!("{:#}", launch_with(&fixture, &factory).await);
    assert!(error.contains("barrier observed"), "{error}");
    assert!(client.authored() >= 1);
    assert_eq!(
        read_all(&chain),
        before,
        "a refuted repair publishes nothing"
    );
    let log = std::fs::read_to_string(fixture.tasks.join(".decompose.log")).unwrap();
    assert!(
        log.contains("frozen_chain_launch acceptance=reauthor ids=AC-X-001"),
        "{log}"
    );
    assert!(
        log.contains("frozen_chain_launch acceptance=reauthor_refuted_routed ids=AC-X-001"),
        "{log}"
    );
    assert!(
        !log.contains("frozen_chain_launch acceptance=reauthor_failed"),
        "{log}"
    );
}

#[path = "workflow_decompose_frozen_chain_refutation_script_test.rs"]
mod refutation_script_test;

#[tokio::test]
async fn a_publisher_failure_after_contract_rename_rolls_back_and_releases_the_claim() {
    let fixture = frozen(Some(names_live_root));
    let mut chain = with_skeleton(&fixture);
    let body = fixture.tasks.join("TASK-X-010.md");
    std::fs::write(&body, "frozen body\n").unwrap();
    chain.push(body);
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);
    let before = read_all(&chain);
    crate::command::workflow_task_set::fail_after_rename(
        &fixture.tasks.join(ACCEPTANCE_CONTRACT_FILE),
    );
    let (factory, _) = scripted_factory(true);
    let error = format!("{:#}", launch_with(&fixture, &factory).await);
    assert!(
        error.contains("injected publisher failure after rename"),
        "{error}"
    );
    assert_eq!(
        read_all(&chain),
        before,
        "the failed publish left a partial chain"
    );
    let run = fixture.store.list_runs().unwrap().pop().unwrap();
    let release = fixture
        .store
        .run_dir(&run.id)
        .join("decomposition/task-root-released.json");
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(release).unwrap()).unwrap();
    assert_eq!(record["run_id"], run.id);
    assert_eq!(
        record["task_root"],
        fixture.tasks.to_string_lossy().replace('\\', "/")
    );
}
