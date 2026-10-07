//! Executor regressions: every child evidence channel crosses one boundary.
use super::*;
use archon_workflow::{PreparedPublicationEntry, PreparedPublicationV1, RunStatus, WorkflowError};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct EvidenceProcess {
    raw: Vec<u8>,
    stderr: String,
    code: i32,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl HostCommandProcessAdapter for EvidenceProcess {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        _: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if (-3..0).contains(&self.code) {
            return Err(match self.code {
                -1 => WorkflowError::StageFailed(self.stderr.clone()),
                -2 => WorkflowError::ControlPaused(self.stderr.clone()),
                _ => WorkflowError::ControlCancelled(self.stderr.clone()),
            });
        }
        let mut entries = Vec::new();
        for path in &request.declared_write_set {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let bytes = if path.file_name().unwrap() == "gate-envelope.json" {
                self.raw.as_slice()
            } else if self.code < -3 {
                self.stderr.as_bytes()
            } else {
                CANDIDATE.as_bytes()
            };
            std::fs::write(path, bytes).unwrap();
            entries.push(PreparedPublicationEntry {
                relative_path: path.file_name().unwrap().to_str().unwrap().into(),
                byte_len: bytes.len() as u64,
                blake3: archon_workflow::task_set_contract::content_digest(bytes),
            });
        }
        entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        let call_id = request
            .args
            .windows(2)
            .find(|p| p[0] == "--call-id")
            .unwrap()[1]
            .clone();
        let stdout = serde_json::to_vec(&PreparedPublicationV1 {
            schema_version: 1,
            call_id,
            command_id: request.command_id,
            entries,
        })
        .unwrap();
        Ok(SupervisedProcessOutput {
            exit_code: Some(if self.code < -3 { 0 } else { self.code }),
            timed_out: false,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: self.stderr.len() as u64,
            stdout,
            stderr: self.stderr.as_bytes().to_vec(),
        })
    }
}
async fn evidence(
    raw: &str,
    stderr: &str,
    code: i32,
    name: &str,
    secret: &str,
) -> (Ran, usize, RunStatus) {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    context.freeze_provider_environment = [(name.into(), secret.into())].into();
    context.acceptance_environment_allowlist = vec![name.into()];
    let task = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task, "live-before").unwrap();
    seed_frozen_chain(&context, &task);
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let mut run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "boundary".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    run.status = RunStatus::Running;
    store.save_state(&run).unwrap();
    let root = store.run_dir(&run.id);
    context.run_staging_root = root.join("host-command-staging");
    let process = Arc::new(EvidenceProcess {
        raw: raw.as_bytes().to_vec(),
        stderr: stderr.into(),
        code,
        calls: AtomicUsize::new(0),
    });
    let executor = FixedHostCommandExecutor::with_process(
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        root.clone(),
        process.clone(),
    );
    let request = HostCommandRequest::new("land-task-body", Some(CANDIDATE.into())).unwrap();
    let id = executor.call_identity(&request).unwrap();
    let staged = root
        .join("host-command-staging")
        .join(&id)
        .join("gate-envelope.json");
    let envelope = root
        .join("host-command-results")
        .join(id)
        .join("gate-envelope.json");
    let result = executor.execute(request, Some(run.generation)).await;
    let status = store.load_state(&run.id).unwrap().status;
    (
        Ran {
            temp,
            result,
            staged,
            envelope,
        },
        process.calls.load(Ordering::SeqCst),
        status,
    )
}
fn clean(ran: &Ran, needles: &[&str]) {
    let returned = match &ran.result {
        Ok(result) => format!(
            "{}\n{}\n{}",
            result.stdout,
            result.stderr,
            serde_json::to_string(result).unwrap()
        ),
        Err(error) => error.to_string(),
    };
    for needle in needles {
        assert!(
            !returned.contains(needle),
            "credential reached returned evidence"
        );
        assert!(
            clear_copies(ran.temp.path(), needle.as_bytes()).is_empty(),
            "credential persisted: {needle}"
        );
    }
}
async fn canonical(report: &str) {
    let raw = format!("{{\"schema_version\":1,\"report\":{report}}}");
    for code in [0, 1, 79] {
        let (ran, _, _) = evidence(&raw, "", code, "SERVICE_PASSWORD", "87654321").await;
        clean(&ran, &["87654321"]);
        let bytes = std::fs::read(if code == 0 {
            &ran.envelope
        } else {
            &ran.staged
        })
        .unwrap();
        let typed: archon_workflow::GateEnvelopeV1 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            bytes,
            serde_json::to_vec_pretty(&typed).unwrap(),
            "unchecked child syntax persisted"
        );
        if code == 0 {
            let result = ran.result.as_ref().unwrap();
            let manifest: PreparedPublicationV1 = serde_json::from_str(&result.stdout).unwrap();
            let entry = manifest
                .entries
                .iter()
                .find(|entry| entry.relative_path == "gate-envelope.json")
                .unwrap();
            assert_eq!(entry.byte_len, bytes.len() as u64);
            assert_eq!(
                entry.blake3,
                archon_workflow::task_set_contract::content_digest(&bytes)
            );
            let receipt = result.publication_receipt.as_ref().unwrap();
            assert!(
                crate::command::workflow_host_command_postcondition::receipt_matches_live(Some(
                    receipt
                ))
                .unwrap()
            );
        }
    }
}
#[tokio::test]
async fn duplicate_keys_cross_executor_boundary() {
    canonical("{\"pin\":87654321,\"pin\":0}").await;
}
#[tokio::test]
async fn exponent_tokens_cross_executor_boundary() {
    canonical("{\"pin\":87654321e-20}").await;
}
#[tokio::test]
async fn nested_duplicate_keys_cross_executor_boundary() {
    canonical("{\"nested\":[{\"pin\":87654321,\"pin\":false}]}").await;
}
async fn operational(secret: &str, printed: &str) {
    let stderr = format!("archon-unsettled-publish: restoring {printed} failed");
    let (ran, calls, status) = evidence(
        "{\"schema_version\":1,\"report\":{}}",
        &stderr,
        79,
        "SERVICE_PASSWORD",
        secret,
    )
    .await;
    clean(&ran, &[secret, printed]);
    assert!(matches!(ran.result, Err(WorkflowError::ControlPaused(_))));
    assert_eq!(calls, 1);
    assert_eq!(status, RunStatus::Paused);
}
#[tokio::test]
async fn exit_79_literal_evidence_crosses_executor_boundary() {
    operational("p4ss", "p4ss").await;
}
#[tokio::test]
async fn exit_79_escaped_evidence_crosses_executor_boundary() {
    operational("p\"4ss", "p\\\"4ss").await;
}
#[tokio::test]
async fn exit_79_unicode_evidence_crosses_executor_boundary() {
    operational("é", "\\u00e9").await;
}
async fn progress(number: &str) {
    let stderr = format!("archon-host-progress: {number}");
    let (ran, calls, status) = evidence(
        "{\"schema_version\":1,\"report\":{}}",
        &stderr,
        75,
        "SERVICE_PASSWORD",
        "87654321",
    )
    .await;
    clean(&ran, &["87654321"]);
    assert!(matches!(ran.result, Err(WorkflowError::ControlPaused(_))));
    assert_eq!(calls, 2);
    assert_eq!(status, RunStatus::Paused);
}
#[tokio::test]
async fn exact_progress_scalar_crosses_executor_boundary() {
    progress("87654321").await;
}
#[tokio::test]
async fn prefixed_progress_scalar_crosses_executor_boundary() {
    progress("187654321").await;
}
#[tokio::test]
async fn suffixed_progress_scalar_crosses_executor_boundary() {
    progress("876543210").await;
}
async fn query(url: &str, encoded: &str, decoded: &str) {
    let raw = serde_json::json!({"schema_version":1, "report":[url, encoded, decoded]}).to_string();
    for code in [0, 1, 79] {
        let stderr = if code == 79 {
            format!("archon-unsettled-publish: restoring {decoded} failed")
        } else {
            format!("{url} {encoded} {decoded}")
        };
        let (ran, _, _) = evidence(&raw, &stderr, code, "DATABASE_URL", url).await;
        clean(&ran, &[url, encoded, decoded]);
        match code {
            0 => assert!(ran.result.as_ref().unwrap().publication_receipt.is_some()),
            1 => assert_eq!(ran.result.as_ref().unwrap().exit_code, Some(1)),
            _ => assert!(matches!(ran.result, Err(WorkflowError::ControlPaused(_)))),
        }
    }
}
#[tokio::test]
async fn query_password_url_crosses_executor_boundary() {
    query(
        "postgresql://db/app?user=alice&password=hunter2",
        "hunter2",
        "hunter2",
    )
    .await;
}
#[tokio::test]
async fn encoded_query_password_crosses_executor_boundary() {
    query(
        "postgresql://db/app?password=hunter%402",
        "hunter%402",
        "hunter@2",
    )
    .await;
}
#[tokio::test]
async fn encoded_query_name_and_plus_password_cross_executor_boundary() {
    query(
        "postgresql://db/app?%70assWORD=hunter+2#fragment",
        "hunter+2",
        "hunter 2",
    )
    .await;
}

async fn returned_error(code: i32, printed: &str) {
    let (ran, _, _) = evidence("{}", printed, code, "SERVICE_PASSWORD", "p4ss").await;
    clean(&ran, &["p4ss"]);
    assert!(ran.result.is_err());
}
#[tokio::test]
async fn child_failure_error_crosses_executor_boundary() {
    returned_error(-1, "refused p4ss").await;
}
#[tokio::test]
async fn child_pause_error_crosses_executor_boundary() {
    returned_error(-2, "paused p4ss").await;
}
#[tokio::test]
async fn child_cancel_error_crosses_executor_boundary() {
    returned_error(-3, "cancelled p4ss").await;
}

async fn marker_collision(secret: &str) {
    let ran = run_secret(Child::Failed, secret).await;
    clean(&ran, &[secret]);
    assert_eq!(ran.result.as_ref().unwrap().exit_code, Some(1));
}
#[tokio::test]
async fn replacement_word_collision_cannot_escape_boundary() {
    marker_collision("REDACTED").await;
}
#[tokio::test]
async fn replacement_marker_collision_cannot_escape_boundary() {
    marker_collision("[REDACTED]").await;
}
#[tokio::test]
async fn replacement_substring_collision_cannot_escape_boundary() {
    marker_collision("RED").await;
}

async fn artifact(payload: &str, secret: &str) {
    let (ran, _, _) = evidence(
        "{\"schema_version\":1,\"report\":{}}",
        payload,
        -4,
        "SERVICE_PASSWORD",
        secret,
    )
    .await;
    let escaped = serde_json::to_string(secret).unwrap();
    clean(&ran, &[secret, &escaped[1..escaped.len() - 1]]);
    let result = ran
        .result
        .as_ref()
        .expect("unsafe artifacts produce operational evidence");
    assert!(result.publication_receipt.is_none());
    assert!(
        result
            .gate_envelope
            .as_ref()
            .unwrap()
            .operational_error
            .is_some()
    );
}
#[tokio::test]
async fn non_envelope_literal_artifacts_cross_executor_boundary() {
    artifact("project credential hunter2", "hunter2").await;
}
#[tokio::test]
async fn non_envelope_json_escaped_artifacts_cross_executor_boundary() {
    artifact(r#"{"credential":"p\"4ss"}"#, "p\"4ss").await;
}
#[tokio::test]
async fn non_envelope_numeric_tokens_cross_executor_boundary() {
    artifact(r#"{"pin":87654321e-20}"#, "87654321").await;
}
