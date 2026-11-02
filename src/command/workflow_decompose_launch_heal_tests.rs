use super::*;

struct RecoveringFactory {
    calls: std::sync::atomic::AtomicUsize,
    client: std::sync::Mutex<Option<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>>,
}

#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for RecoveringFactory {
    async fn build_client(
        &self,
        _request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>
    {
        let attempt = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if attempt < 3 {
            Err(archon_workflow::WorkflowError::port(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset,
            )))
        } else {
            self.client
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| archon_workflow::WorkflowError::port("barrier observed"))
        }
    }
}

#[tokio::test]
async fn workflow_decompose_launch_repair_recovers_post_publish_read_without_reauthoring_after_transient_construction()
 {
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };
    let fixture = frozen(Some(names_live_root));
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);
    let client = std::sync::Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f marker"),
        |_, _| true,
    ));
    let factory = RecoveringFactory {
        calls: Default::default(),
        client: std::sync::Mutex::new(Some(client.clone())),
    };
    crate::command::workflow_decompose_launch_repair::heal::READ_FAILURE
        .set(Some(std::io::ErrorKind::Interrupted));
    let error = format!(
        "{:#}",
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            launch_with(&fixture, &factory)
        )
        .await
        .expect("launch recovery must finish")
    );
    assert!(error.contains("barrier observed"), "{error}");
    assert_eq!(factory.calls.load(std::sync::atomic::Ordering::SeqCst), 5);
    assert_eq!(client.authored(), 1);
    let log = std::fs::read_to_string(fixture.tasks.join(".decompose.log")).unwrap();
    assert!(
        log.contains(
            "acceptance=reauthored published_ids=AC-X-001 pending_ids= acceptance_digest="
        ),
        "{log}"
    );
}

#[tokio::test]
async fn workflow_decompose_launch_repair_recovers_post_publish_read_without_reauthoring() {
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };
    let fixture = frozen(Some(names_live_root));
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);
    let client = std::sync::Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f marker"),
        |_, _| true,
    ));
    let factory = ReauthorThenBarrier {
        reauthor: std::sync::Mutex::new(Some(client.clone())),
        origins: Default::default(),
    };
    crate::command::workflow_decompose_launch_repair::heal::READ_FAILURE
        .set(Some(std::io::ErrorKind::Interrupted));
    let error = format!(
        "{:#}",
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            launch_with(&fixture, &factory)
        )
        .await
        .expect("launch recovery must finish")
    );
    assert!(error.contains("barrier observed"), "{error}");
    assert_eq!(client.authored(), 1);
    let snapshot = crate::command::workflow_decompose_frozen_chain::frozen_chain_snapshot(
        &fixture.project,
        &fixture.project.join("prds/PRD-X.md"),
        &fixture.tasks,
    )
    .unwrap();
    assert!(snapshot.acceptance);
}

struct InterruptedJudge {
    inner: std::sync::Arc<
        crate::command::workflow_task_set::reauthor::test_client::ScriptedAuthorJudge,
    >,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for InterruptedJudge {
    fn provider_id(&self) -> Option<String> {
        self.inner.provider_id()
    }
    fn resolve_model_alias(&self, model: &str) -> String {
        self.inner.resolve_model_alias(model)
    }

    async fn run_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.inner.run_agent(call).await
    }
    async fn continue_agent(
        &self,
        call: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        self.inner.continue_agent(call).await
    }
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        unreachable!("judges use the sampled dispatch")
    }
    async fn send_message_with_temperature(
        &self,
        messages: Vec<serde_json::Value>,
        system: Vec<serde_json::Value>,
        tools: Vec<serde_json::Value>,
        model: &str,
        temperature: f64,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 3 {
            return Err(archon_workflow::WorkflowError::port(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset,
            )));
        }
        self.inner
            .send_message_with_temperature(messages, system, tools, model, temperature)
            .await
    }
}
struct JudgeRepairFactory(std::sync::Arc<InterruptedJudge>);
#[async_trait::async_trait(?Send)]
impl archon_workflow::WorkflowLlmClientFactory for JudgeRepairFactory {
    async fn build_client(
        &self,
        request: archon_workflow::WorkflowLlmClientRequest,
    ) -> archon_workflow::WorkflowResult<std::sync::Arc<dyn archon_workflow::WorkflowLlmClient>>
    {
        if request.origin == "workflow-decompose-launch-reauthor" {
            Ok(self.0.clone())
        } else {
            Err(archon_workflow::WorkflowError::port("barrier observed"))
        }
    }
}
#[tokio::test]
async fn workflow_decompose_launch_repair_task_set_error_transient_judge_recovers_successfully() {
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };
    let fixture = frozen(Some(names_live_root));
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);
    let client = std::sync::Arc::new(InterruptedJudge {
        inner: std::sync::Arc::new(ScriptedAuthorJudge::new(
            |entry, _| command_entry(entry, "test -f marker"),
            |_, _| true,
        )),
        calls: Default::default(),
    });
    let error = format!(
        "{:#}",
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            launch_with(&fixture, &JudgeRepairFactory(client.clone()))
        )
        .await
        .expect("judge recovery must finish")
    );
    assert!(error.contains("barrier observed"), "{error}");
    assert!(client.calls.load(std::sync::atomic::Ordering::SeqCst) >= 4);
    let log = std::fs::read_to_string(fixture.tasks.join(".decompose.log")).unwrap();
    assert!(
        log.contains(
            "acceptance=reauthored published_ids=AC-X-001 pending_ids= acceptance_digest="
        ),
        "{log}"
    );
}

async fn committed_repair_fault(point: crate::command::workflow_host_fault_test_hooks::Point) {
    use crate::command::workflow_host_fault_test_hooks::inject;
    use crate::command::workflow_task_set::reauthor::test_client::{
        ScriptedAuthorJudge, command_entry,
    };
    let fixture = frozen(Some(names_live_root));
    crate::command::workflow_task_set::passability_tests_support::commit_project(&fixture.project);
    let client = std::sync::Arc::new(ScriptedAuthorJudge::new(
        |entry, _| command_entry(entry, "test -f marker"),
        |_, _| true,
    ));
    let factory = ReauthorThenBarrier {
        reauthor: std::sync::Mutex::new(Some(client.clone())),
        origins: Default::default(),
    };
    let pin_path =
        crate::command::workflow_task_set::acceptance_pin_path(&fixture.project, &fixture.tasks);
    let prior: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    let committed = std::sync::Arc::new(std::sync::Mutex::new(None));
    let saved = committed.clone();
    let read_pin = pin_path.clone();
    let path = if point == crate::command::workflow_host_fault_test_hooks::Point::CommitSync {
        let pin = crate::command::workflow_task_set::acceptance_pin_path(
            &fixture.project,
            &fixture.tasks,
        );
        archon_workflow::task_set_publish_lock::journal_paths(&pin)[0].clone()
    } else {
        fixture.tasks.clone()
    };
    let fault = inject(
        point,
        &path,
        Box::new(move || {
            *saved.lock().unwrap() = Some(std::fs::read(&read_pin).unwrap());
            Err(std::io::ErrorKind::Interrupted.into())
        }),
    );
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        launch_with(&fixture, &factory),
    )
    .await
    .expect("repair must settle");
    assert!(fault.fired());
    assert!(format!("{error:#}").contains("barrier observed"));
    assert_eq!(
        client.authored(),
        1,
        "committed repair must not author or publish again"
    );
    let final_bytes = std::fs::read(&pin_path).unwrap();
    assert_eq!(
        final_bytes,
        committed.lock().unwrap().take().unwrap(),
        "the committed publication identity must be retained exactly"
    );
    let final_pin: archon_workflow::task_set_contract::AcceptancePin =
        serde_json::from_slice(&final_bytes).unwrap();
    assert_eq!(final_pin.lineage.len(), prior.lineage.len() + 1);
    let log = std::fs::read_to_string(fixture.tasks.join(".decompose.log")).unwrap();
    assert!(
        log.contains(
            "acceptance=reauthored published_ids=AC-X-001 pending_ids= acceptance_digest="
        ),
        "{log}"
    );
}
#[tokio::test]
async fn workflow_decompose_launch_commit_flush_recovers_without_republishing() {
    committed_repair_fault(crate::command::workflow_host_fault_test_hooks::Point::CommitSync).await;
}
#[tokio::test]
async fn workflow_decompose_launch_archive_recovers_without_republishing() {
    committed_repair_fault(crate::command::workflow_host_fault_test_hooks::Point::Archive).await;
}
