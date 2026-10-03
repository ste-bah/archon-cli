//! Secret values a host-command child holds, kept out of what its call records.
//!
//! A child's stdout, stderr and gate envelope are persisted with the call. The
//! child runs with the configured acceptance allowlist and, for provider
//! profiles, the provider credentials, and anything it prints or reports can
//! carry those values. They are replaced at this boundary, by exact value.
//! PATH, HOME and the other process essentials are not secrets and stay as
//! they are; they never enter an identity or digest in the first place.
use std::collections::BTreeMap;
use std::ffi::OsString;

use archon_workflow::{GateEnvelopeV1, WorkflowError, WorkflowResult};

use super::workflow_host_command_catalog::HostCommandResolutionContext;

/// Provider settings that are configuration, not credentials.
const PROVIDER_SETTINGS: &[&str] = &["ARCHON_MODEL", "ARCHON_EFFORT", "ARCHON_CONFIG_DIR"];
/// Shorter values are not credentials, and replacing them would only garble
/// ordinary output.
const MIN_SECRET_LEN: usize = 8;
pub(crate) const REDACTED: &str = "[REDACTED]";

pub(crate) fn utf8(bytes: Vec<u8>, stream: &str) -> WorkflowResult<String> {
    String::from_utf8(bytes).map_err(|error| {
        WorkflowError::StageFailed(format!("host command {stream} is not UTF-8: {error}"))
    })
}

pub(crate) struct HostSecrets(Vec<String>);

impl HostSecrets {
    /// The allowlisted and provider-credential values the child was given.
    pub(crate) fn of(
        context: &HostCommandResolutionContext,
        environment: &BTreeMap<String, OsString>,
    ) -> Self {
        let mut values = context
            .acceptance_environment_allowlist
            .iter()
            .chain(context.freeze_provider_environment.keys())
            .filter(|name| !PROVIDER_SETTINGS.contains(&name.as_str()))
            .filter_map(|name| environment.get(name)?.to_str().map(str::to_string))
            .filter(|value| value.len() >= MIN_SECRET_LEN)
            .collect::<Vec<_>>();
        // Longest first, so a value containing another is replaced whole.
        values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        values.dedup();
        Self(values)
    }

    pub(crate) fn text(&self, text: &str) -> String {
        self.0.iter().fold(text.to_string(), |text, value| {
            text.replace(value.as_str(), REDACTED)
        })
    }

    /// Every string in the envelope, by value; its structure is untouched.
    pub(crate) fn envelope(&self, envelope: GateEnvelopeV1) -> WorkflowResult<GateEnvelopeV1> {
        if self.0.is_empty() {
            return Ok(envelope);
        }
        let mut value = serde_json::to_value(envelope)?;
        self.strings(&mut value);
        Ok(serde_json::from_value(value)?)
    }

    fn strings(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => *text = self.text(text),
            serde_json::Value::Array(items) => items.iter_mut().for_each(|item| self.strings(item)),
            serde_json::Value::Object(fields) => {
                fields.values_mut().for_each(|field| self.strings(field))
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(allowlist: &[&str], provider: &[(&str, &str)]) -> HostCommandResolutionContext {
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
        let clean = HostSecrets::of(&context, &environment)
            .envelope(envelope.clone())
            .unwrap();
        let encoded = serde_json::to_string(&clean).unwrap();
        assert!(!encoded.contains("allowlisted-secret"), "{encoded}");
        assert!(encoded.contains(REDACTED), "{encoded}");
        assert_eq!(clean.report["count"], 1);
        assert_eq!(
            clean.policy_findings[0].remediation_scope,
            envelope.policy_findings[0].remediation_scope
        );
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
                    timed_out: false,
                    stdout_bytes: stdout.len() as u64,
                    stderr_bytes: stderr.len() as u64,
                    stdout,
                    stderr,
                },
            )
        }
    }

    #[test]
    fn host_command_result_never_holds_an_allowlisted_value() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
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
            super::super::workflow_host_command_catalog::fixed_decomposition_catalog("rev-1")
                .unwrap(),
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
}
