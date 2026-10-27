use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract,
    AcceptanceLock, AcceptancePin, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE,
    content_digest,
};
use archon_workflow::task_skeleton::{TaskSkeleton, TaskSkeletonLock};
use async_trait::async_trait;
use serde_json::Value;

use super::tests::{Fixture, execution};
use super::*;

#[derive(Default)]
struct Judge {
    calls: AtomicUsize,
    prompts: Mutex<Vec<String>>,
    author_reply: Mutex<Option<String>>,
    parser_refutes: AtomicBool,
}

#[async_trait]
impl archon_workflow::WorkflowLlmClient for Judge {
    fn provider_id(&self) -> Option<String> {
        Some("scripted".into())
    }

    async fn run_agent(
        &self,
        call: archon_workflow::llm_client_port::WorkflowAgentCall,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        if let Some(content) = self.author_reply.lock().unwrap().clone() {
            return Ok(WorkflowAgentOutcome {
                content,
                stop_reason: Some("end_turn".into()),
                ..WorkflowAgentOutcome::default()
            });
        }
        let entry: Value = call
            .task
            .lines()
            .find_map(|line| line.strip_prefix("The entry being replaced: "))
            .map(|line| serde_json::from_str(line).unwrap())
            .expect("the re-author request includes its entry");
        let replacement = "test -f absent";
        Ok(WorkflowAgentOutcome {
            content: crate::command::workflow_task_set::reauthor::test_client::command_entry(&entry, &replacement),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }

    async fn send_message(
        &self,
        messages: Vec<Value>,
        system: Vec<Value>,
        tools: Vec<Value>,
        model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.send_message_with_temperature(messages, system, tools, model, 0.0)
            .await
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<Value>,
        _system: Vec<Value>,
        _tools: Vec<Value>,
        _model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert_eq!(temperature, 0.0);
        self.calls.fetch_add(1, Ordering::SeqCst);
        let prompt = messages[0]["content"].as_str().unwrap();
        self.prompts.lock().unwrap().push(prompt.to_string());
        let checks: Vec<Value> = serde_json::from_str(
            prompt
                .split_once("Checks: ")
                .map_or("[]", |(_, checks)| checks),
        )
        .unwrap_or_default();
        let decisions: Vec<Value> = checks
            .iter()
            .map(|check| {
                let output = check["baseline"]["stderr"].as_str().unwrap_or_default();
                let refuted = output.contains("below required minimum")
                    || (self.parser_refutes.load(Ordering::SeqCst)
                        && output.contains("unrecognized argument"));
                // A correct judge quotes the refusing line exactly as printed (#526).
                let rule = if output.contains("below required minimum") { "below required minimum" } else { "unrecognized argument" };
                let line = output.lines().find(|line| line.contains(rule)).unwrap_or_default().trim();
                serde_json::json!({
                    "id": check["id"],
                    "verdict": if refuted { "refuted" } else { "accepted" },
                    "counterexample": if refuted { format!("the output says \"{line}\"") } else { "no setup rule refused the check".to_string() },
                    "reason": if refuted { "the check supplies too little data for the required minimum" } else { "the output shows the feature is absent" }
                })
            })
            .collect();
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({"decisions": decisions}).to_string(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }
}

fn with_check(mut fixture: Fixture, command: &str) -> Fixture {
    let prd_text = "# PRD\n\n| ID | Acceptance criterion |\n| --- | --- |\n| AC-X-001 | feature output is valid |\n";
    std::fs::write(fixture.project.path().join("prd.md"), prd_text).unwrap();
    let path = fixture.task_root.join(ACCEPTANCE_CONTRACT_FILE);
    let mut contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    contract.prd.digest = content_digest(prd_text.as_bytes());
    contract.acceptance = vec![super::tests::criterion("AC-X-001", command)];
    contract.acceptance[0].criterion = "feature output is valid".into();
    contract.acceptance[0].judgment.sampling =
        Some(serde_json::json!({"model": "model-x", "provider": "scripted"}));
    contract.supplementary.clear();
    let bytes = serde_json::to_vec_pretty(&contract).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let acceptance_digest = content_digest(&bytes);
    let lock_path = fixture.task_root.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.digest = acceptance_digest.clone();
    std::fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    let skeleton_path = fixture.task_root.join(TASK_SKELETON_FILE);
    let mut skeleton: TaskSkeleton =
        serde_json::from_slice(&std::fs::read(&skeleton_path).unwrap()).unwrap();
    let old_files: Vec<_> = skeleton
        .tasks
        .iter()
        .map(|task| task.file_name.clone())
        .collect();
    skeleton.acceptance_digest = acceptance_digest.clone();
    skeleton.tasks[0].task_id = "TASK-X-001".into();
    skeleton.tasks[0].file_name = "TASK-X-001.md".into();
    skeleton.tasks[0].implements = vec!["AC-X-001".into()];
    skeleton.tasks[1].task_id = "TASK-X-002".into();
    skeleton.tasks[1].file_name = "TASK-X-002.md".into();
    skeleton.tasks[1].implements.clear();
    let skeleton_bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    std::fs::write(&skeleton_path, &skeleton_bytes).unwrap();
    let skeleton_digest = content_digest(&skeleton_bytes);
    let skeleton_lock_path = fixture.task_root.join(TASK_SKELETON_LOCK_FILE);
    let mut skeleton_lock: TaskSkeletonLock =
        serde_json::from_slice(&std::fs::read(&skeleton_lock_path).unwrap()).unwrap();
    skeleton_lock.digest = skeleton_digest.clone();
    skeleton_lock.acceptance_digest = acceptance_digest.clone();
    std::fs::write(
        &skeleton_lock_path,
        serde_json::to_vec_pretty(&skeleton_lock).unwrap(),
    )
    .unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(
        fixture.project.path(),
        &fixture.task_root,
    );
    let mut pin: AcceptancePin = serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.acceptance_digest = acceptance_digest;
    pin.skeleton_digest = Some(skeleton_digest);
    std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
    for (index, id) in ["TASK-X-001", "TASK-X-002"].iter().enumerate() {
        let task_path = fixture.task_root.join(format!("{id}.md"));
        std::fs::write(
            task_path,
            format!(
                "# {id}\n\n```yaml\ntask_id: {id}\ntitle: Test task\ncomplexity: low\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: {}\nrequired_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n",
                if index == 0 { "[AC-X-001]" } else { "[]" }
            ),
        )
        .unwrap();
        fixture.universe.tasks[index].canonical_task_id = (*id).into();
        fixture.universe.tasks[index].source_path = fixture
            .task_root
            .join(format!("{id}.md"))
            .display()
            .to_string();
    }
    for file in old_files {
        std::fs::remove_file(fixture.task_root.join(file)).unwrap();
    }
    fixture.universe.tasks[0].implements = vec!["AC-X-001".into()];
    fixture.universe.tasks[1].implements.clear();
    fixture
}

fn without_recorded_model(fixture: &Fixture) {
    let contract_path = fixture.task_root.join(ACCEPTANCE_CONTRACT_FILE);
    let mut contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(&contract_path).unwrap()).unwrap();
    contract.acceptance[0].judgment.sampling = None;
    let contract_bytes = serde_json::to_vec_pretty(&contract).unwrap();
    std::fs::write(&contract_path, &contract_bytes).unwrap();
    let acceptance_digest = content_digest(&contract_bytes);
    let lock_path = fixture.task_root.join(ACCEPTANCE_LOCK_FILE);
    let mut lock: AcceptanceLock = serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
    lock.digest = acceptance_digest.clone();
    std::fs::write(lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    let skeleton_path = fixture.task_root.join(TASK_SKELETON_FILE);
    let mut skeleton: TaskSkeleton = serde_json::from_slice(&std::fs::read(&skeleton_path).unwrap()).unwrap();
    skeleton.acceptance_digest = acceptance_digest.clone();
    let skeleton_bytes = serde_json::to_vec_pretty(&skeleton).unwrap();
    std::fs::write(&skeleton_path, &skeleton_bytes).unwrap();
    let skeleton_digest = content_digest(&skeleton_bytes);
    let skeleton_lock_path = fixture.task_root.join(TASK_SKELETON_LOCK_FILE);
    let mut skeleton_lock: TaskSkeletonLock = serde_json::from_slice(&std::fs::read(&skeleton_lock_path).unwrap()).unwrap();
    skeleton_lock.digest = skeleton_digest.clone();
    skeleton_lock.acceptance_digest = acceptance_digest.clone();
    std::fs::write(skeleton_lock_path, serde_json::to_vec_pretty(&skeleton_lock).unwrap()).unwrap();
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(fixture.project.path(), &fixture.task_root);
    let mut pin: AcceptancePin = serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    pin.acceptance_digest = acceptance_digest;
    pin.skeleton_digest = Some(skeleton_digest);
    std::fs::write(pin_path, serde_json::to_vec_pretty(&pin).unwrap()).unwrap();
}

async fn until_stall(fixture: &Fixture, client: &Judge) -> (u32, String) {
    until_stall_with_rounds(fixture, client, 5).await
}

async fn until_stall_with_rounds(
    fixture: &Fixture,
    client: &Judge,
    max_rounds: u32,
) -> (u32, String) {
    for round in 1..=max_rounds {
        let execution = execution(round, max_rounds, &[]);
        let generation = fixture.store.load_state(&fixture.run_id).unwrap().generation;
        match run_acceptance_stage(
            &fixture.runtime,
            &execution,
            &fixture.store,
            &fixture.run_id,
            archon_workflow::control_pause::PauseOwner::Generation(generation),
            Some(&fixture.universe),
            Some(client),
        )
        .await
        {
            Err(archon_workflow::WorkflowError::ControlPaused(message)) => {
                return (round, message)
            }
            Ok(_) => {}
            Err(error) => panic!("unexpected acceptance error: {error}"),
        }
    }
    panic!("the stalled round did not pause")
}

#[tokio::test]
async fn a_stalled_accepted_check_refuted_on_round_evidence_is_reauthored() {
    let fixture = with_check(
        super::tests::fixture(true),
        "echo 'fixture observed rows below required minimum' >&2; exit 1",
    );
    let client = Judge::default();
    for round in 1..=5 {
        let execution = execution(round, 5, &[]);
        let generation = fixture.store.load_state(&fixture.run_id).unwrap().generation;
        let outcome = run_acceptance_stage(
            &fixture.runtime,
            &execution,
            &fixture.store,
            &fixture.run_id,
            archon_workflow::control_pause::PauseOwner::Generation(generation),
            Some(&fixture.universe),
            Some(&client),
        )
        .await;
        let result = match outcome {
            Ok(result) => result,
            Err(error) => {
                let record = archon_workflow::v2::acceptance_stage::latest_round_record(
                    &fixture.store.run_dir(&fixture.run_id),
                )
                .unwrap()
                .unwrap()
                .0;
                panic!("unexpected acceptance error {error}; checks={:?}", record.checks);
            }
        };
        if let Some(repairs) = result.data["contract_repairs"]
            .as_array()
            .filter(|repairs| !repairs.is_empty())
        {
            assert!(
                repairs.iter().any(|repair| repair["repaired"] == true),
                "refutation reached the repair path but did not publish: {repairs:?}"
            );
            assert_eq!(result.data["failing"][0]["check_id"], "AC-X-001");
            assert!(client.calls.load(Ordering::SeqCst) > 0);
            return;
        }
    }
    panic!("the refuted stalled check was not re-authored within the exercised rounds");
}

#[tokio::test]
async fn an_accepted_stalled_check_is_named_with_its_verdict_and_audit_reuses_it() {
    let fixture = with_check(super::tests::fixture(true), "echo feature-missing >&2; exit 1");
    let client = Judge::default();
    let (round, message) = until_stall(&fixture, &client).await;
    assert!(message.contains("AC-X-001"), "{message}");
    assert!(message.contains("AC-X-001: Accepted"), "{message}");
    let judged = client.calls.load(Ordering::SeqCst);
    assert!(judged > 0);
    let lifecycle = archon_workflow::LifecycleController::new(fixture.store.clone());
    lifecycle
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    let generation = fixture.store.load_state(&fixture.run_id).unwrap().generation;
    let resumed = execution(round, 5, &[]);
    let _ = run_acceptance_stage(
        &fixture.runtime,
        &resumed,
        &fixture.store,
        &fixture.run_id,
        archon_workflow::control_pause::PauseOwner::Generation(generation),
        Some(&fixture.universe),
        Some(&client),
    )
    .await;
    assert_eq!(client.calls.load(Ordering::SeqCst), judged, "prompts: {:?}", client.prompts.lock().unwrap());
}

struct RestorePath(Option<std::ffi::OsString>);

impl Drop for RestorePath {
    fn drop(&mut self) {
        unsafe {
            if let Some(path) = self.0.take() {
                std::env::set_var("PATH", path);
            } else {
                std::env::remove_var("PATH");
            }
        }
    }
}

#[tokio::test]
#[serial_test::serial]
async fn a_script_invocation_with_external_parser_output_stays_on_the_judge_path() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = with_check(
        super::tests::fixture(true),
        "sh scripts/check.sh",
    );
    let script_dir = fixture.repo.path().join("scripts");
    std::fs::create_dir_all(&script_dir).unwrap();
    std::fs::write(script_dir.join("check.sh"), "fakecheck --badflag\n").unwrap();
    for args in [
        vec!["add", "scripts/check.sh"],
        vec!["commit", "-m", "add check script"],
    ] {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(fixture.repo.path())
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
    let external = tempfile::tempdir().unwrap();
    let program = external.path().join("fakecheck");
    std::fs::write(
        &program,
        "#!/bin/sh\necho 'fakecheck: error: unrecognized argument: --badflag' >&2\necho 'Usage: fakecheck [OPTIONS]' >&2\nexit 2\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&program).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&program, permissions).unwrap();
    let old_path = std::env::var_os("PATH");
    let _restore = RestorePath(old_path.clone());
    let mut paths = vec![external.path().to_path_buf()];
    if let Some(old_path) = old_path {
        paths.extend(std::env::split_paths(&old_path));
    }
    unsafe { std::env::set_var("PATH", std::env::join_paths(paths).unwrap()) };
    let client = Judge::default();
    let (_, message) = until_stall(&fixture, &client).await;
    assert!(message.contains("AC-X-001: Accepted"), "{message}");
    assert!(client.calls.load(Ordering::SeqCst) > 0);
    assert!(client
        .prompts
        .lock()
        .unwrap()
        .iter()
        .any(|prompt| prompt.contains("unrecognized argument: --badflag")));
    let repairs = archon_workflow::v2::acceptance_stage::latest_round_record(
        &fixture.store.run_dir(&fixture.run_id),
    ).unwrap().unwrap().0.contract_repairs;
    assert!(repairs.iter().all(|repair| repair.repaired == false));
}

#[tokio::test]
async fn a_stalled_silent_check_is_refuted_without_calling_the_judge() {
    let fixture = with_check(super::tests::fixture(true), "test -f absent");
    let client = Judge::default();
    for round in 1..=5 {
        let execution = execution(round, 5, &[]);
        let generation = fixture.store.load_state(&fixture.run_id).unwrap().generation;
        let result = run_acceptance_stage(
            &fixture.runtime, &execution, &fixture.store, &fixture.run_id,
            archon_workflow::control_pause::PauseOwner::Generation(generation),
            Some(&fixture.universe), Some(&client),
        ).await;
        if let Ok(result) = result {
            let repairs = result.data["contract_repairs"].as_array().unwrap();
            if !repairs.is_empty() {
                assert_eq!(repairs[0]["repaired"], true, "{repairs:?}");
                assert!(client.calls.load(Ordering::SeqCst) <= 2, "silent check reached the judge path: {:?}", client.prompts.lock().unwrap());
                return;
            }
        }
    }
    let record = archon_workflow::v2::acceptance_stage::latest_round_record(
        &fixture.store.run_dir(&fixture.run_id),
    ).unwrap().unwrap().0;
    panic!("silent failed check did not reach host re-authoring: {record:?}");
}

#[tokio::test]
async fn stalled_refutation_without_base_corroboration_pauses_with_the_check_named() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = with_check(super::tests::fixture(true), "sh scripts/check.sh");
    std::fs::create_dir_all(fixture.repo.path().join("scripts")).unwrap();
    std::fs::write(fixture.repo.path().join("scripts/check.sh"), "test -f trigger && fakecheck --badflag\n").unwrap();
    for args in [vec!["add", "scripts/check.sh"], vec!["commit", "-m", "add check script"]] {
        let output = std::process::Command::new("git").arg("-C").arg(fixture.repo.path()).args(args).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
    std::fs::write(fixture.repo.path().join("trigger"), "present").unwrap();
    let external = tempfile::tempdir().unwrap();
    let program = external.path().join("fakecheck");
    std::fs::write(&program, "#!/bin/sh\necho 'fakecheck: error: unrecognized argument: --badflag' >&2\necho 'Usage: fakecheck [OPTIONS]' >&2\nexit 2\n").unwrap();
    let mut permissions = std::fs::metadata(&program).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&program, permissions).unwrap();
    let old_path = std::env::var_os("PATH");
    let _restore = RestorePath(old_path.clone());
    let mut paths = vec![external.path().to_path_buf()];
    if let Some(old_path) = old_path { paths.extend(std::env::split_paths(&old_path)); }
    unsafe { std::env::set_var("PATH", std::env::join_paths(paths).unwrap()) };
    let client = Judge::default();
    client.parser_refutes.store(true, Ordering::SeqCst);
    let (_, message) = until_stall(&fixture, &client).await;
    assert!(message.contains("AC-X-001") && message.contains("not corroborated on the base tree"), "{message}");
    assert!(client.calls.load(Ordering::SeqCst) > 0, "implemented-tree parser evidence is judged");
}

#[tokio::test]
async fn a_reauthor_reply_that_cannot_be_published_is_reported_at_the_pause() {
    let fixture = with_check(super::tests::fixture(true), "echo 'fixture observed rows below required minimum' >&2; exit 1");
    let client = Judge::default();
    *client.author_reply.lock().unwrap() = Some("not a valid authored entry".into());
    let (round, message) = until_stall(&fixture, &client).await;
    assert!(message.contains("republish") || message.contains("re-author"), "{message}");
    assert!(message.contains("AC-X-001"), "{message}");
    let judged = client.calls.load(Ordering::SeqCst);
    assert!(judged > 0, "the refutation was judged before re-authoring");
    archon_workflow::LifecycleController::new(fixture.store.clone())
        .apply(&fixture.run_id, archon_workflow::LifecycleAction::Resume)
        .unwrap();
    let generation = fixture.store.load_state(&fixture.run_id).unwrap().generation;
    let _ = run_acceptance_stage(
        &fixture.runtime, &execution(round, 5, &[]), &fixture.store, &fixture.run_id,
        archon_workflow::control_pause::PauseOwner::Generation(generation),
        Some(&fixture.universe), Some(&client),
    ).await;
    assert_eq!(client.calls.load(Ordering::SeqCst), judged, "same evidence caused another judge call");
}

#[tokio::test]
async fn stalled_pause_reports_missing_judge_model_and_context_resolution_errors() {
    let fixture = with_check(super::tests::fixture(true), "test -f absent");
    without_recorded_model(&fixture);
    let client = Judge::default();
    let (_, message) = until_stall(&fixture, &client).await;
    assert!(message.contains("no recorded judge model"), "{message}");

    let mut broken = with_check(super::tests::fixture(true), "test -f absent");
    let missing_root = broken.project.path().join("missing-repository");
    broken.runtime.target_repository_root = Some(missing_root.display().to_string());
    let (_, message) = until_stall(&broken, &Judge::default()).await;
    assert!(message.contains(&missing_root.display().to_string()), "{message}");
    assert!(message.contains("acceptance stage"), "{message}");
}
