use super::*;

pub(super) fn context(
    allowlist: &[&str],
    provider: &[(&str, &str)],
) -> HostCommandResolutionContext {
    let root = std::path::Path::new("/project");
    HostCommandResolutionContext {
        program: root.join("archon"),
        project_root: root.into(),
        prd_path: root.join("prd.md"),
        prd_digest: "digest".into(),
        task_root: root.join("tasks"),
        run_staging_root: root.join("staging"),
        frozen_task_id: None,
        frozen_task_file: None,
        freeze_provider_environment: provider
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
        acceptance_environment_allowlist: allowlist.iter().map(|n| n.to_string()).collect(),
        gate_mode: archon_core::config::GateMode::Observe,
    }
}

#[test]
fn allowlisted_and_credential_values_are_redacted_but_essentials_are_not() {
    let context = context(
        &["DATA_PROVIDER_KEY"],
        &[
            ("ANTHROPIC_API_KEY", "sk-provider-credential"),
            ("ARCHON_MODEL", "provider-model-name"),
        ],
    );
    let environment = BTreeMap::from([
        (
            "PATH".to_string(),
            OsString::from("/usr/local/toolchain/bin"),
        ),
        ("HOME".to_string(), OsString::from("/home/operator-account")),
        ("DATA_PROVIDER_KEY".into(), "allowlisted-secret".into()),
        ("ANTHROPIC_API_KEY".into(), "sk-provider-credential".into()),
        ("ARCHON_MODEL".into(), "provider-model-name".into()),
    ]);
    let secrets = HostSecrets::of(&context, &environment);
    let printed = "key=allowlisted-secret token=sk-provider-credential \
         path=/usr/local/toolchain/bin home=/home/operator-account model=provider-model-name";
    let clean = secrets.text(printed);
    assert!(!clean.contains("allowlisted-secret"), "{clean}");
    assert!(!clean.contains("sk-provider-credential"), "{clean}");
    for kept in [
        "/usr/local/toolchain/bin",
        "/home/operator-account",
        "provider-model-name",
    ] {
        assert!(clean.contains(kept), "{kept} lost: {clean}");
    }
}

#[test]
fn spilled_child_output_is_redacted_before_it_is_read_as_a_result() {
    let context = context(&["DATA_PROVIDER_KEY"], &[]);
    let secrets = HostSecrets::of(
        &context,
        &BTreeMap::from([(
            "DATA_PROVIDER_KEY".into(),
            OsString::from("allowlisted-secret"),
        )]),
    );
    let temp = tempfile::tempdir().unwrap();
    let spill = temp.path().join("stdout.bin");
    std::fs::write(&spill, b"prefix allowlisted-secret suffix").unwrap();

    secrets.redact_spill(&spill).unwrap();

    let stored = std::fs::read_to_string(spill).unwrap();
    assert!(!stored.contains("allowlisted-secret"), "{stored}");
    assert!(stored.contains(REDACTED), "{stored}");
}

#[test]
fn streamed_spill_redaction_catches_a_secret_crossing_a_chunk_boundary() {
    let context = context(&["DATA_PROVIDER_KEY"], &[]);
    let secrets = HostSecrets::of(
        &context,
        &BTreeMap::from([(
            "DATA_PROVIDER_KEY".into(),
            OsString::from("allowlisted-secret"),
        )]),
    );
    let temp = tempfile::tempdir().unwrap();
    let spill = temp.path().join("stdout.bin");
    let mut bytes = vec![b'x'; 64 * 1024 - 8];
    bytes.push(b' ');
    bytes.extend_from_slice(b"allowlisted-secret");
    bytes.extend_from_slice(b" tail");
    std::fs::write(&spill, bytes).unwrap();

    secrets.redact_spill(&spill).unwrap();

    let stored = std::fs::read_to_string(spill).unwrap();
    assert!(
        !stored.contains("allowlisted-secret"),
        "secret crossed the read boundary: {stored}"
    );
    assert!(stored.contains(REDACTED), "{stored}");
    assert!(stored.ends_with(" tail"), "{stored}");
}

#[test]
fn envelope_strings_are_redacted_and_its_structure_kept() {
    let context = context(&["DATA_PROVIDER_KEY"], &[]);
    let environment = BTreeMap::from([(
        "DATA_PROVIDER_KEY".into(),
        OsString::from("allowlisted-secret"),
    )]);
    let envelope: GateEnvelopeV1 = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "report": {"lines": ["probe printed allowlisted-secret"], "count": 1},
        "policy_findings": [{"text": "check saw allowlisted-secret", "subject": "T1",
            "remediation_scope": "body"}],
        "operational_error": {"kind": "probe", "text": "allowlisted-secret leaked"}
    }))
    .unwrap();
    let clean = HostSecrets::of(&context, &environment).envelope(envelope.clone());
    let encoded = serde_json::to_string(&clean).unwrap();
    assert!(!encoded.contains("allowlisted-secret"), "{encoded}");
    assert!(encoded.contains(REDACTED), "{encoded}");
    assert_eq!(clean.report["count"], 1);
    assert_eq!(
        clean.policy_findings[0].remediation_scope,
        envelope.policy_findings[0].remediation_scope
    );
}

#[test]
fn non_secret_allowlisted_keyword_keeps_valid_envelope() {
    let context = context(&["RUN_MODE", "DATA_PROVIDER_KEY"], &[]);
    let environment = BTreeMap::from([
        ("RUN_MODE".into(), OsString::from("operational")),
        (
            "DATA_PROVIDER_KEY".into(),
            OsString::from("credential-canary"),
        ),
    ]);
    let envelope: GateEnvelopeV1 = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "report": "operational credential-canary",
        "policy_findings": [{"text": "operational credential-canary", "subject": "T1",
            "remediation_scope": "operational"}]
    }))
    .unwrap();
    let clean = HostSecrets::of(&context, &environment).envelope(envelope.clone());
    assert_eq!(clean.report, "operational [REDACTED]");
    assert_eq!(clean.policy_findings[0].text, "operational [REDACTED]");
    assert_eq!(
        clean.policy_findings[0].remediation_scope,
        envelope.policy_findings[0].remediation_scope
    );
}

#[test]
fn secret_keyword_redacts_free_text_and_preserves_identifiers() {
    let context = context(&["access_tOkEn"], &[]);
    let environment = BTreeMap::from([("access_tOkEn".into(), OsString::from("operational"))]);
    let envelope: GateEnvelopeV1 = serde_json::from_value(serde_json::json!({
        "schema_version": 1,
        "report": {"lines": ["operational"], "count": 1},
        "policy_findings": [{"text": "operational", "subject": "operational",
            "source_path": "operational", "remediation_scope": "operational"}],
        "operational_error": {"kind": "operational", "text": "operational"}
    }))
    .unwrap();
    let clean = HostSecrets::of(&context, &environment).envelope(envelope.clone());
    assert_eq!(clean.report["lines"][0], REDACTED);
    assert_eq!(clean.report["count"], 1);
    let finding = &clean.policy_findings[0];
    assert_eq!(finding.text, REDACTED);
    assert_eq!(finding.subject, REDACTED);
    assert_eq!(finding.source_path.as_deref(), Some(REDACTED));
    assert_eq!(
        finding.remediation_scope,
        envelope.policy_findings[0].remediation_scope
    );
    let error = clean.operational_error.unwrap();
    assert_eq!(error.kind, REDACTED);
    assert_eq!(error.text, REDACTED);
}

struct MalformedOutputProcess {
    envelope: bool,
}

#[async_trait::async_trait]
impl super::super::workflow_host_command_exec::HostCommandProcessAdapter
    for MalformedOutputProcess
{
    async fn execute(
        &self,
        request: super::super::workflow_host_command_catalog::ResolvedHostCommand,
        _control: super::super::workflow_host_command_supervisor::HostCommandControl,
    ) -> WorkflowResult<super::super::workflow_host_command_supervisor::SupervisedProcessOutput>
    {
        let secret = request.environment["ANTHROPIC_API_KEY"].to_str().unwrap();
        let malformed = serde_json::to_vec(&serde_json::json!({"schema_version": secret})).unwrap();
        let stdout = if self.envelope {
            let path = request
                .declared_write_set
                .iter()
                .find(|path| path.file_name().unwrap() == "gate-envelope.json")
                .unwrap();
            std::fs::write(path, &malformed).unwrap();
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 1, "call_id": "test", "command_id": request.command_id,
                "entries": [{"relative_path": "gate-envelope.json",
                    "byte_len": malformed.len(),
                    "blake3": archon_workflow::task_set_contract::content_digest(&malformed)}]
            }))
            .unwrap()
        } else {
            malformed
        };
        Ok(
            super::super::workflow_host_command_supervisor::SupervisedProcessOutput {
                exit_code: Some(0),
                stdout_bytes: stdout.len() as u64,
                stderr_bytes: 0,
                stdout_retained_bytes: stdout.len() as u64,
                stderr_retained_bytes: 0,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout,
                stderr: Vec::new(),
                timed_out: false,
                stdout_path: None,
                stderr_path: None,
            },
        )
    }
}

async fn malformed_output_error(envelope: bool) -> String {
    use super::super::workflow_host_command_exec::{
        FixedHostCommandExecutor, WorkflowHostCommandExecutor,
    };
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let prd = project.join("prd.md");
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    std::fs::write(&prd, "# PRD\n").unwrap();
    let mut context = context(&[], &[("ANTHROPIC_API_KEY", "parser-credential-canary")]);
    context.project_root = project.clone();
    context.prd_digest =
        archon_workflow::task_set_contract::content_digest(&std::fs::read(&prd).unwrap());
    context.prd_path = prd;
    context.task_root = project.join("tasks");
    let store = archon_workflow::WorkflowStore::project(&project);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-parser-secrets".into(),
            task: "redaction".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let executor = FixedHostCommandExecutor::with_process(
        super::super::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        run_root,
        std::sync::Arc::new(MalformedOutputProcess { envelope }),
    );
    let request = archon_workflow::HostCommandRequest::new("task-set-lint", None).unwrap();
    executor
        .execute(request, Some(run.generation))
        .await
        .unwrap_err()
        .to_string()
}

#[tokio::test]
async fn malformed_manifest_error_redacts_secret() {
    let error = malformed_output_error(false).await;
    assert!(error.contains("malformed prepared manifest"), "{error}");
    assert!(!error.contains("parser-credential-canary"), "{error}");
    assert!(error.contains(REDACTED), "{error}");
}

#[tokio::test]
async fn malformed_envelope_error_redacts_secret() {
    let error = malformed_output_error(true).await;
    assert!(error.contains("invalid type"), "{error}");
    assert!(!error.contains("parser-credential-canary"), "{error}");
    assert!(error.contains(REDACTED), "{error}");
}

/// Prints the allowlisted value it was given and fails, as a child that
/// echoes its environment into an error would.
struct EchoingProcess;

#[async_trait::async_trait]
impl super::super::workflow_host_command_exec::HostCommandProcessAdapter for EchoingProcess {
    async fn execute(
        &self,
        request: super::super::workflow_host_command_catalog::ResolvedHostCommand,
        _control: super::super::workflow_host_command_supervisor::HostCommandControl,
    ) -> WorkflowResult<super::super::workflow_host_command_supervisor::SupervisedProcessOutput>
    {
        let value = request.environment["ARCHON_274_SECRET"].to_str().unwrap();
        let stdout = format!("connecting with {value}").into_bytes();
        let stderr = format!("rejected {value}").into_bytes();
        Ok(
            super::super::workflow_host_command_supervisor::SupervisedProcessOutput {
                exit_code: Some(1),
                stdout_bytes: stdout.len() as u64,
                stderr_bytes: stderr.len() as u64,
                stdout_retained_bytes: stdout.len() as u64,
                stderr_retained_bytes: stderr.len() as u64,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout,
                stderr,
                timed_out: false,
                stdout_path: None,
                stderr_path: None,
            },
        )
    }
}

#[test]
fn host_command_result_never_holds_an_allowlisted_value() {
    let status = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "command::workflow_host_secrets::tests::echoing_child",
            "--nocapture",
        ])
        .env("ARCHON_274_SECRET", "allowlisted-secret-canary")
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
#[ignore = "isolated process environment"]
async fn echoing_child() {
    use super::super::workflow_host_command_exec::{
        FixedHostCommandExecutor, WorkflowHostCommandExecutor,
    };
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let prd = project.join("prd.md");
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    std::fs::write(&prd, "# PRD\n").unwrap();
    let mut context = context(&["ARCHON_274_SECRET"], &[]);
    context.project_root = project.clone();
    context.prd_digest =
        archon_workflow::task_set_contract::content_digest(&std::fs::read(&prd).unwrap());
    context.prd_path = prd;
    context.task_root = project.join("tasks");
    let store = archon_workflow::WorkflowStore::project(&project);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-secrets".into(),
            task: "redaction".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let run_root = store.run_dir(&run.id);
    context.run_staging_root = run_root.join("host-command-staging");
    let executor = FixedHostCommandExecutor::with_process(
        super::super::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        run_root,
        std::sync::Arc::new(EchoingProcess),
    );
    let request = archon_workflow::HostCommandRequest::new("task-set-lint", None).unwrap();
    let result = executor
        .execute(request, Some(run.generation))
        .await
        .unwrap();
    let recorded = serde_json::to_string(&result).unwrap();
    assert!(
        !recorded.contains("allowlisted-secret-canary"),
        "{recorded}"
    );
    assert_eq!(result.stdout, format!("connecting with {REDACTED}"));
    assert_eq!(result.stderr, format!("rejected {REDACTED}"));
}
