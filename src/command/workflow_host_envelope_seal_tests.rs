//! Issue 277: a host-command child that prints a provider credential into its
//! gate envelope leaves no clear copy of it anywhere under the run.

use std::path::{Path, PathBuf};

use archon_workflow::{HostCommandRequest, HostCommandResult, WorkflowResult};

use super::workflow_host_command_catalog::{ResolvedHostCommand, fixed_decomposition_catalog};
use super::workflow_host_command_exec::{
    FixedHostCommandExecutor, HostCommandProcessAdapter, WorkflowHostCommandExecutor,
};
use super::workflow_host_command_exec_tests::{context, seed_frozen_chain};
use super::workflow_host_command_supervisor::{HostCommandControl, SupervisedProcessOutput};

const CANARY: &str = "sk-ant-277-canary-5f1e9c";

const CANDIDATE: &str = "# Candidate\n\n```yaml\ntask_id: TASK-X-010\ntitle: Candidate\n\
complexity: low\nstatus: ready\ndepends_on: []\nblocks: []\nimplements: []\n\
required_env_keys: []\nrequired_tools: []\ndeliverable_contracts: []\n```\n\n\
## Focused Tests\n- `test -f TASK-X-010.md`\n";

#[derive(Clone, Copy, PartialEq)]
enum Child {
    /// A valid envelope whose report (and an unknown field) holds the value.
    Published,
    /// The same envelope with no secret in it.
    Clean,
    /// The value in the envelope's operational error text.
    Operational,
    /// The envelope written, then a failing exit.
    Failed,
    /// Bytes that are not an envelope at all.
    Malformed,
    /// A manifest that misstates the envelope's bytes.
    Lying,
    SealedIdentity,
    CanonicalIdentity,
    SecretKey,
    Interrupted,
    Paused,
    Cancelled,
    TypedSecret,
    OperationalTypedSecret,
}

struct SecretPrintingProcess(Child);

fn envelope(child: Child, secret: &str) -> Vec<u8> {
    let value = match child {
        Child::Malformed => return format!("not an envelope {secret}").into_bytes(),
        Child::OperationalTypedSecret => serde_json::json!({
            "schema_version": 1, "report": "probe", "operational_error": {"kind": secret, "text": "failure"}
        }),
        Child::TypedSecret => serde_json::json!({"schema_version": 1, "report": "failure",
            "policy_findings": [{"text":"failure", "subject":secret, "remediation_scope":"body"}]}),
        Child::SecretKey => serde_json::json!({"schema_version": 1, "report": {secret: "failure"}}),
        Child::Clean => serde_json::json!({"schema_version": 1, "report": "body accepted"}),
        Child::Operational => serde_json::json!({
            "schema_version": 1, "report": "probe",
            "operational_error": {"kind": "probe", "text": format!("provider refused {secret}")}
        }),
        _ => serde_json::json!({
            "schema_version": 1,
            "report": {"lines": [format!("probe printed {secret}")], "count": 1},
            "debug_environment": secret,
        }),
    };
    serde_json::to_vec_pretty(&value).unwrap()
}

#[async_trait::async_trait]
impl HostCommandProcessAdapter for SecretPrintingProcess {
    async fn execute(
        &self,
        request: ResolvedHostCommand,
        _control: HostCommandControl,
    ) -> WorkflowResult<SupervisedProcessOutput> {
        use archon_workflow::task_set_contract::content_digest;
        use archon_workflow::{
            PREPARED_PUBLICATION_SCHEMA_VERSION, PreparedPublicationEntry, PreparedPublicationV1,
        };
        let secret = request.environment["ANTHROPIC_API_KEY"].to_str().unwrap();
        let mut entries = Vec::new();
        for path in &request.declared_write_set {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let bytes = if name == "gate-envelope.json" {
                envelope(self.0, secret)
            } else {
                CANDIDATE.as_bytes().to_vec()
            };
            if matches!(
                self.0,
                Child::Interrupted | Child::Paused | Child::Cancelled
            ) && name == "gate-envelope.json"
            {
                std::fs::write(path.with_extension("interrupted.tmp"), &bytes).unwrap();
                return Err(match self.0 {
                    Child::Paused => archon_workflow::WorkflowError::ControlPaused("paused".into()),
                    Child::Cancelled => {
                        archon_workflow::WorkflowError::ControlCancelled("cancelled".into())
                    }
                    _ => archon_workflow::WorkflowError::StageFailed("interrupted".into()),
                });
            }
            std::fs::write(path, &bytes).unwrap();
            let digest = if self.0 == Child::Lying && name == "gate-envelope.json" {
                content_digest(b"something else")
            } else {
                content_digest(&bytes)
            };
            let claimed = if matches!(self.0, Child::SealedIdentity | Child::CanonicalIdentity)
                && name == "gate-envelope.json"
            {
                claimed_envelope(self.0)
            } else {
                bytes.clone()
            };
            entries.push(PreparedPublicationEntry {
                relative_path: name,
                byte_len: claimed.len() as u64,
                blake3: if matches!(self.0, Child::SealedIdentity | Child::CanonicalIdentity) {
                    content_digest(&claimed)
                } else {
                    digest
                },
            });
        }
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let call_id = (request.args.windows(2))
            .find(|pair| pair[0] == "--call-id")
            .map(|pair| pair[1].clone())
            .unwrap();
        let stdout = serde_json::to_vec(&PreparedPublicationV1 {
            schema_version: PREPARED_PUBLICATION_SCHEMA_VERSION,
            call_id,
            command_id: request.command_id,
            entries,
        })
        .unwrap();
        let stderr = format!("warning: key {secret} in use").into_bytes();
        Ok(SupervisedProcessOutput {
            exit_code: Some(if self.0 == Child::Failed { 1 } else { 0 }),
            timed_out: false,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: stderr.len() as u64,
            stdout,
            stderr,
        })
    }
}

fn claimed_envelope(child: Child) -> Vec<u8> {
    let envelope: archon_workflow::GateEnvelopeV1 =
        serde_json::from_slice(&envelope(Child::Published, "[REDACTED]")).unwrap();
    if child == Child::CanonicalIdentity {
        serde_json::to_vec_pretty(&serde_json::to_value(envelope).unwrap()).unwrap()
    } else {
        serde_json::to_vec_pretty(&envelope).unwrap()
    }
}

struct Ran {
    temp: tempfile::TempDir,
    result: WorkflowResult<HostCommandResult>,
    envelope: PathBuf,
    staged: PathBuf,
}

async fn run(child: Child) -> Ran {
    run_secret(child, CANARY).await
}

async fn run_secret(child: Child, secret: &str) -> Ran {
    let temp = tempfile::tempdir().unwrap();
    let mut context = context(temp.path());
    context.freeze_provider_environment = [("ANTHROPIC_API_KEY".into(), secret.into())].into();
    let task_file = context.task_root.join("TASK-X-010.md");
    std::fs::write(&task_file, b"live-before").unwrap();
    seed_frozen_chain(&context, &task_file);
    let store = archon_workflow::WorkflowStore::project(&context.project_root);
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
            name: "host-envelope-seal".into(),
            task: "seal".into(),
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
        fixed_decomposition_catalog("rev-1").unwrap(),
        context,
        run_root.clone(),
        std::sync::Arc::new(SecretPrintingProcess(child)),
    );
    let request = HostCommandRequest::new("land-task-body", Some(CANDIDATE.into())).unwrap();
    let call_id = executor.call_identity(&request).unwrap();
    let staged = run_root
        .join("host-command-staging")
        .join(&call_id)
        .join("gate-envelope.json");
    let result = executor.execute(request, Some(run.generation)).await;
    let envelope = (run_root.join("host-command-results").join(call_id)).join("gate-envelope.json");
    Ran {
        temp,
        result,
        envelope,
        staged,
    }
}

/// Every file under `root` (the run directory, the staging tree, the
/// published results, the project), searched byte for byte.
fn clear_copies(root: &Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            found.extend(clear_copies(&path, needle));
        } else if kind.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            if bytes.windows(needle.len()).any(|window| window == needle) {
                found.push(path);
            }
        }
    }
    found
}

fn assert_no_clear_copy(ran: &Ran) {
    let found = clear_copies(ran.temp.path(), CANARY.as_bytes());
    assert!(found.is_empty(), "secret persisted in clear: {found:?}");
}

#[tokio::test]
async fn published_envelope_is_redacted_and_its_receipt_still_verifies() {
    let ran = run(Child::Published).await;
    let result = ran.result.as_ref().expect("published");
    assert_no_clear_copy(&ran);
    let receipt = result.publication_receipt.as_ref().expect("receipt");
    assert!(
        super::workflow_host_command_postcondition::receipt_matches_live(Some(receipt)).unwrap()
    );
    let published: archon_workflow::GateEnvelopeV1 =
        serde_json::from_slice(&std::fs::read(&ran.envelope).unwrap()).unwrap();
    assert_eq!(published.report["lines"][0], "probe printed [REDACTED]");
    assert_eq!(published.report["count"], 1, "structure kept");
    assert_eq!(Some(&published), result.gate_envelope.as_ref());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&ran.envelope)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "published child output is owner-only");
    }
}

#[tokio::test]
async fn an_envelope_without_secrets_is_published_byte_for_byte() {
    let ran = run(Child::Clean).await;
    ran.result.as_ref().expect("published");
    assert_eq!(
        std::fs::read(&ran.envelope).unwrap(),
        envelope(Child::Clean, CANARY),
        "nothing to redact, nothing rewritten"
    );
}

#[tokio::test]
async fn an_operational_envelope_left_in_staging_is_redacted() {
    let ran = run(Child::Operational).await;
    let result = ran.result.as_ref().expect("unpublished outcome");
    assert!(result.publication_receipt.is_none());
    assert!(!ran.envelope.exists());
    assert_no_clear_copy(&ran);
}

#[tokio::test]
async fn a_failed_child_leaves_no_clear_envelope() {
    let ran = run(Child::Failed).await;
    let result = ran.result.as_ref().expect("failed outcome");
    assert_eq!(result.exit_code, Some(1));
    assert!(!result.stderr.contains(CANARY));
    assert_no_clear_copy(&ran);
}

#[tokio::test]
async fn a_malformed_envelope_is_redacted_and_still_refused() {
    let ran = run(Child::Malformed).await;
    let error = ran.result.as_ref().expect_err("malformed").to_string();
    assert!(!error.contains(CANARY), "{error}");
    assert_no_clear_copy(&ran);
}

#[tokio::test]
async fn a_manifest_that_misstates_the_envelope_is_still_refused() {
    let ran = run(Child::Lying).await;
    let error = ran
        .result
        .as_ref()
        .expect_err("refused by audit")
        .to_string();
    // The manifest named other bytes than the child wrote, so it is never
    // rebound to the sealed ones: the audit refuses the staged envelope.
    assert!(
        error.contains("staged output gate-envelope.json") && error.contains("mismatch"),
        "{error}"
    );
    assert!(!ran.envelope.exists(), "nothing published");
    assert_no_clear_copy(&ran);
}

#[tokio::test]
async fn round2_json_escaped_credentials_are_sealed() {
    for secret in [
        "credential-quote\"-canary",
        "credential-backslash\\-canary",
        "credential-newline\n-canary",
    ] {
        let ran = run_secret(Child::Published, secret).await;
        ran.result.as_ref().unwrap();
        let escaped = serde_json::to_string(secret).unwrap();
        for needle in [secret.as_bytes(), &escaped.as_bytes()[1..escaped.len() - 1]] {
            assert!(
                clear_copies(ran.temp.path(), needle).is_empty(),
                "{secret:?}"
            );
        }
    }
}

#[tokio::test]
async fn round2_object_keys_are_sealed() {
    let ran = run(Child::SecretKey).await;
    ran.result.as_ref().unwrap();
    assert_no_clear_copy(&ran);
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ran.envelope).unwrap()).unwrap();
    assert_eq!(value["report"]["[REDACTED]"], "failure");
}

#[tokio::test]
async fn round2_interrupted_temporary_envelopes_are_removed() {
    for child in [Child::Interrupted, Child::Paused, Child::Cancelled] {
        let ran = run(child).await;
        assert!(ran.result.is_err());
        assert_no_clear_copy(&ran);
    }
}

#[tokio::test]
async fn round2_manifest_claiming_sealed_identity_is_refused() {
    for child in [Child::SealedIdentity, Child::CanonicalIdentity] {
        let ran = run(child).await;
        assert!(ran.result.is_err(), "raw mismatch accepted");
        assert!(!ran.envelope.exists());
        assert_no_clear_copy(&ran);
        if child == Child::CanonicalIdentity {
            assert_eq!(
                std::fs::read(&ran.staged).unwrap(),
                claimed_envelope(child),
                "the dishonest manifest names exactly the current sealed bytes"
            );
        }
    }
}

#[tokio::test]
async fn round2_residual_typed_field_is_sealed_fail_closed() {
    for child in [Child::TypedSecret, Child::OperationalTypedSecret] {
        let ran = run(child).await;
        assert!(
            ran.result.is_err(),
            "redaction must not erase a typed failure field"
        );
        assert_no_clear_copy(&ran);
    }
}
