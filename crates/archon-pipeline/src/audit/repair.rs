use std::fs;

use anyhow::Result;

use super::runtime::PipelineAuditRun;
use super::types::PipelineEvent;

impl PipelineAuditRun {
    pub fn pending_artifact_repair(
        &self,
        agent_key: &str,
        validator_key: &str,
    ) -> Result<Option<(usize, String)>> {
        let mut pending = None;
        for event in self.read_events()? {
            match event {
                PipelineEvent::ArtifactRepairPending {
                    agent_key: key,
                    validator_key: event_validator_key,
                    attempt,
                    findings_hash,
                    findings,
                } if key == agent_key => {
                    anyhow::ensure!(
                        event_validator_key == validator_key,
                        "pending repair validator mismatch for {agent_key}"
                    );
                    let actual_hash = super::store::sha256_hex(findings.as_bytes());
                    anyhow::ensure!(
                        findings_hash == actual_hash,
                        "pending repair findings hash mismatch for {agent_key} attempt {attempt}"
                    );
                    pending = Some((attempt, findings_hash, findings));
                }
                PipelineEvent::ArtifactRepairCommitted {
                    agent_key: key,
                    attempt,
                    ..
                }
                | PipelineEvent::ArtifactRepairRejected {
                    agent_key: key,
                    attempt,
                    ..
                } if key == agent_key
                    && pending
                        .as_ref()
                        .is_some_and(|(pending_attempt, _, _)| *pending_attempt == attempt) =>
                {
                    pending = None
                }
                _ => {}
            }
        }
        if let Some((attempt, _, _)) = &pending {
            let progress_attempt = self.last_validator_attempt(validator_key)?;
            anyhow::ensure!(
                *attempt == progress_attempt,
                "pending repair attempt mismatch for {validator_key}: marker attempt {attempt}, progress attempt {progress_attempt}"
            );
        }
        Ok(pending.map(|(attempt, _, findings)| (attempt, findings)))
    }

    pub fn record_artifact_repair_pending(
        &self,
        validator_key: &str,
        agent_key: &str,
        attempt: usize,
        findings: &str,
    ) -> Result<()> {
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::ArtifactRepairPending {
                agent_key: agent_key.to_string(),
                validator_key: validator_key.to_string(),
                attempt,
                findings_hash: super::store::sha256_hex(findings.as_bytes()),
                findings: findings.to_string(),
            },
        )
    }

    pub(super) fn read_events(&self) -> Result<Vec<PipelineEvent>> {
        let raw = fs::read_to_string(
            self.store
                .bundle_dir(&self.state.session_id)
                .join("audit.log"),
        )?;
        raw.lines()
            .map(|line| {
                Ok(serde_json::from_str::<crate::audit::types::PipelineEventLine>(line)?.event)
            })
            .collect()
    }

    pub fn last_validator_attempt(&self, agent_key: &str) -> Result<usize> {
        Ok(self
            .read_events()?
            .into_iter()
            .filter_map(|event| match event {
                PipelineEvent::ValidatorProgress {
                    agent_key: key,
                    attempt,
                } if key == agent_key => Some(attempt),
                PipelineEvent::ArtifactRepairPending {
                    validator_key: key,
                    attempt,
                    ..
                } if key == agent_key => Some(attempt),
                _ => None,
            })
            .last()
            .unwrap_or(0))
    }

    pub fn persist_artifact_repair(
        &self,
        agent_key: &str,
        attempt: usize,
        ordinal: usize,
        input_hash: String,
        output_hash: String,
        output: &str,
    ) -> Result<()> {
        let path = self.store.write_artifact_repair(
            &self.state.session_id,
            ordinal,
            agent_key,
            attempt,
            output,
        )?;
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::ArtifactRepairCommitted {
                agent_key: agent_key.to_string(),
                attempt,
                output_path: path
                    .strip_prefix(self.store.bundle_dir(&self.state.session_id))?
                    .to_string_lossy()
                    .replace('\\', "/"),
                input_hash,
                output_hash,
            },
        )
    }

    pub fn record_validator_progress(&self, agent_key: &str, attempt: usize) -> Result<()> {
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::ValidatorProgress {
                agent_key: agent_key.to_string(),
                attempt,
            },
        )
    }

    pub fn record_validator_repair_pending(
        &self,
        validator_key: &str,
        artifact_key: &str,
        attempt: usize,
        findings: &str,
    ) -> Result<()> {
        self.record_artifact_repair_pending(validator_key, artifact_key, attempt, findings)
    }
}
