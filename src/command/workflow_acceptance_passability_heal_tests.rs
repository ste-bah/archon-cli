use super::*;
struct Outage(EvidenceJudge);
#[async_trait]
impl WorkflowLlmClient for Outage {
    fn provider_id(&self) -> Option<String> {
        self.0.provider_id()
    }
    async fn send_message_with_temperature(
        &self,
        messages: Vec<Value>,
        system: Vec<Value>,
        tools: Vec<Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        if messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("\"baseline\"")
        {
            return Err(archon_workflow::WorkflowError::port(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset,
            )));
        }
        self.0
            .send_message_with_temperature(messages, system, tools, model, temperature)
            .await
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
}
#[tokio::test]
async fn host_command_freeze_keeps_authored_findings_before_passability_outage() {
    let (project, _outside, tasks, prd) = outside_set(ABSENT);
    let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut draft: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut second = draft["acceptance"][0].clone();
    second["id"] = "SUP-X-001".into();
    draft["acceptance"][0]["check"]["command"] =
        format!("test -f {}/marker", project.path().display()).into();
    draft["supplementary"].as_array_mut().unwrap().push(second);
    let candidate = serde_json::to_vec(&draft).unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        prepare_acceptance_freeze_resumable(
            project.path(),
            &tasks,
            &prd,
            GateMode::Observe,
            candidate,
            Arc::new(Outage(EvidenceJudge::default())),
            &FreezeResume::none(),
        ),
    )
    .await
    .expect("freeze must return authored findings");
    let prepared = result.expect("an independent host outage must not erase an authored defect");
    assert!(
        prepared
            .findings
            .iter()
            .any(|finding| finding.subject == "AC-X-001"
                && finding.remediation_scope != archon_workflow::RemediationScope::Operational),
        "{:?}",
        prepared.findings
    );
}

#[tokio::test]
async fn host_command_freeze_repository_read_keeps_transient_io() {
    let (project, _outside, tasks, prd) = outside_set(ABSENT);
    let candidate = std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    let fault = crate::command::workflow_host_fault_test_hooks::inject(
        crate::command::workflow_host_fault_test_hooks::Point::Repository,
        &tasks,
        Box::new(|| Err(std::io::Error::from_raw_os_error(5))),
    );
    let error = prepare_acceptance_freeze_resumable(
        project.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        candidate,
        Arc::new(EvidenceJudge::default()),
        &FreezeResume::none(),
    )
    .await
    .err()
    .expect("injected read failure");
    assert!(fault.fired());
    assert_eq!(
        crate::command::workflow_host_fault::classify(&error),
        crate::command::workflow_host_fault::Class::Transient,
        "{error:#}"
    );
}

#[tokio::test]
async fn host_command_freeze_returns_cached_refutation_before_unavailable_judgment() {
    let (project, _outside, tasks, prd) = outside_set(UNPASSABLE);
    let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut draft: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut second = draft["acceptance"][0].clone();
    second["id"] = "SUP-X-001".into();
    second["check"]["command"] = "test -f missing-output".into();
    draft["supplementary"].as_array_mut().unwrap().push(second);
    let candidate = serde_json::to_vec(&draft).unwrap();
    let resume = saving();
    let prepared = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        prepare_acceptance_freeze_resumable(
            project.path(),
            &tasks,
            &prd,
            GateMode::Observe,
            candidate.clone(),
            Arc::new(EvidenceJudge::default()),
            &resume,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(cannot_pass(&prepared, "AC-X-001").len(), 1);
    // Change only the undecided check's key; the grounded refutation stays cached.
    draft["supplementary"][0]["criterion"] = "An observable output is present".into();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        prepare_acceptance_freeze_resumable(
            project.path(),
            &tasks,
            &prd,
            GateMode::Observe,
            serde_json::to_vec(&draft).unwrap(),
            Arc::new(Outage(EvidenceJudge::default())),
            &resume,
        ),
    )
    .await
    .expect("known findings must return promptly")
    .expect("cached authored refutation must survive the other judgment's outage");
    assert_eq!(cannot_pass(&result, "AC-X-001").len(), 1);
    assert_eq!(
        result.contract().unwrap().supplementary[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}

struct PartialOutage {
    judge: EvidenceJudge,
    answered: AtomicUsize,
}
#[async_trait]
impl WorkflowLlmClient for PartialOutage {
    fn provider_id(&self) -> Option<String> {
        self.judge.provider_id()
    }
    async fn send_message_with_temperature(
        &self,
        messages: Vec<Value>,
        system: Vec<Value>,
        tools: Vec<Value>,
        model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        let baseline = messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("\"baseline\"");
        if baseline && self.answered.fetch_add(1, SeqCst) > 0 {
            return Err(archon_workflow::WorkflowError::port(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset,
            )));
        }
        let mut outcome = self
            .judge
            .send_message_with_temperature(messages, system, tools, model, temperature)
            .await?;
        if baseline {
            let mut reply: Value = serde_json::from_str(&outcome.content).unwrap();
            reply["decisions"]
                .as_array_mut()
                .unwrap()
                .retain(|entry| entry["id"] == "AC-X-001");
            outcome.content = reply.to_string();
        }
        Ok(outcome)
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
}

#[tokio::test]
async fn host_command_freeze_returns_partial_refutation_despite_remaining_judge_outage() {
    let (project, _outside, tasks, prd) = outside_set(UNPASSABLE);
    let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut draft: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut second = draft["acceptance"][0].clone();
    second["id"] = "SUP-X-001".into();
    second["check"]["command"] = "test -f missing-output".into();
    draft["supplementary"].as_array_mut().unwrap().push(second);
    let client = Arc::new(PartialOutage {
        judge: EvidenceJudge::default(),
        answered: Default::default(),
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        prepare_acceptance_freeze_resumable(
            project.path(),
            &tasks,
            &prd,
            GateMode::Observe,
            serde_json::to_vec(&draft).unwrap(),
            client.clone(),
            &saving(),
        ),
    )
    .await
    .expect("partial refutation must return promptly")
    .expect("partial authored refutation must survive the remaining judgment's outage");
    assert_eq!(
        client.answered.load(SeqCst),
        1,
        "deliver the grounded refutation before another provider call"
    );
    assert_eq!(cannot_pass(&result, "AC-X-001").len(), 1);
    assert_eq!(
        result.contract().unwrap().supplementary[0].judgment.verdict,
        JudgeDecision::Accepted
    );
}

#[tokio::test]
async fn host_command_freeze_repository_lock_missing_during_read_is_permanent() {
    let (project, _outside, tasks, prd) = outside_set(ABSENT);
    let candidate = std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    // A legacy set without a lock may use the project. A lock that disappears
    // during its read must instead retain the permanent repair instruction.
    let fault = crate::command::workflow_host_fault_test_hooks::inject(
        crate::command::workflow_host_fault_test_hooks::Point::Repository,
        &tasks,
        Box::new(|| Err(std::io::Error::from(std::io::ErrorKind::NotFound))),
    );
    let error = prepare_acceptance_freeze_resumable(
        project.path(),
        &tasks,
        &prd,
        GateMode::Observe,
        candidate,
        Arc::new(EvidenceJudge::default()),
        &FreezeResume::none(),
    )
    .await
    .err()
    .expect("missing repository lock during read must require repair");
    assert!(fault.fired());
    assert_eq!(
        crate::command::workflow_host_fault::classify(&error),
        crate::command::workflow_host_fault::Class::Permanent,
        "{error:#}"
    );
    assert!(
        matches!(error.downcast_ref::<archon_workflow::WorkflowError>(),
            Some(archon_workflow::WorkflowError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound),
        "{error:#}"
    );
    let text = format!("{error:#}");
    let lock = archon_workflow::repository_record::repository_record_path(&tasks);
    assert!(text.contains(&format!("restore {} as its decomposition wrote it, or remove the task set and decompose again, then re-run: the task set's repository root is unknown", lock.display())), "{text}");
}
