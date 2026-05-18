use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;

use crate::audit::store::{PipelineBundleStore, json_hash, sha256_hex};
use crate::audit::types::{
    AgentAttemptRecord, AgentAuditRecord, BundleState, BundleStatus, PipelineEvent,
    PromptAuditRecord,
};
use crate::audit::verify::verify_bundle;
use crate::runner::{AgentInfo, AgentResult, PipelineType};

pub struct PipelineAuditRun {
    store: PipelineBundleStore,
    state: BundleState,
}

impl PipelineAuditRun {
    pub fn start(
        worktree: impl AsRef<Path>,
        session_id: &str,
        pipeline_type: PipelineType,
        task: &str,
    ) -> Result<Self> {
        let store = PipelineBundleStore::new(worktree);
        let state = store.create(session_id, pipeline_type, task)?;
        Ok(Self { store, state })
    }

    pub fn resume(worktree: impl AsRef<Path>, session_id: &str) -> Result<Self> {
        let store = PipelineBundleStore::new(worktree);
        let report = verify_bundle(&store, session_id, true)?;
        if !report.valid {
            let details = report
                .findings
                .iter()
                .filter(|finding| finding.severity == "error")
                .map(|finding| finding.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("pipeline bundle failed verification before resume: {details}");
        }

        let mut state = store.load_state(session_id)?;
        state.status = BundleStatus::Running;
        state.current_agent_key = None;
        state.updated_at = Utc::now();
        state.last_error = None;
        store.save_state(&state)?;
        store.append_event(
            session_id,
            PipelineEvent::RunResumed {
                completed_agent_count: state.completed_agent_count,
            },
        )?;
        Ok(Self { store, state })
    }

    pub fn state(&self) -> &BundleState {
        &self.state
    }

    pub fn store(&self) -> &PipelineBundleStore {
        &self.store
    }

    pub fn hydrate_results(&self) -> Result<Vec<(AgentInfo, AgentResult)>> {
        let dir = self.store.bundle_dir(&self.state.session_id);
        let mut results = Vec::new();
        for record in self.store.list_agent_records(&self.state.session_id)? {
            let output = fs::read_to_string(dir.join(&record.output_path))
                .with_context(|| format!("read output for {}", record.agent_key))?;
            let agent = AgentInfo {
                key: record.agent_key,
                display_name: record.display_name,
                model: record.requested_model,
                phase: record.phase,
                critical: record.critical,
                parallelizable: false,
                quality_threshold: record.quality_threshold,
                tool_access_level: record.tool_access_level,
            };
            let result = AgentResult {
                output,
                tool_use_log: record.tool_use_log,
                tokens_in: record.tokens_in,
                tokens_out: record.tokens_out,
                cost_usd: record.cost_usd,
                duration: Duration::from_millis(record.duration_ms),
                quality: record.quality,
            };
            results.push((agent, result));
        }
        Ok(results)
    }

    pub fn record_agent_planned(&mut self, ordinal: usize, agent: &AgentInfo) -> Result<()> {
        self.state.current_agent_key = Some(agent.key.clone());
        self.state.updated_at = Utc::now();
        self.store.save_state(&self.state)?;
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::AgentPlanned {
                ordinal,
                agent_key: agent.key.clone(),
                phase: agent.phase,
            },
        )
    }

    pub fn record_prompt(
        &self,
        ordinal: usize,
        agent: &AgentInfo,
        messages: &[serde_json::Value],
        system: &[serde_json::Value],
        tools: &[serde_json::Value],
    ) -> Result<PromptHashes> {
        let hashes = PromptHashes {
            prompt_record_path: String::new(),
            messages_hash: json_hash(&messages)?,
            system_hash: json_hash(&system)?,
            tools_hash: json_hash(&tools)?,
        };
        let record = PromptAuditRecord {
            ordinal,
            agent_key: agent.key.clone(),
            messages_hash: hashes.messages_hash.clone(),
            system_hash: hashes.system_hash.clone(),
            tools_hash: hashes.tools_hash.clone(),
            messages: messages.to_vec(),
            system: system.to_vec(),
            tools: tools.to_vec(),
            created_at: Utc::now(),
        };
        let path = self.store.write_prompt(&self.state.session_id, &record)?;
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::PromptBuilt {
                ordinal,
                agent_key: agent.key.clone(),
                messages_hash: hashes.messages_hash.clone(),
                system_hash: hashes.system_hash.clone(),
                tools_hash: hashes.tools_hash.clone(),
            },
        )?;
        Ok(PromptHashes {
            prompt_record_path: relative_path(&self.store, &self.state.session_id, &path)?,
            ..hashes
        })
    }

    pub fn record_attempt_started(
        &self,
        ordinal: usize,
        agent: &AgentInfo,
        attempt: usize,
    ) -> Result<()> {
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::LlmAttemptStarted {
                ordinal,
                agent_key: agent.key.clone(),
                attempt,
                model: agent.model.clone(),
            },
        )
    }

    pub fn record_attempt_completed(
        &self,
        ordinal: usize,
        agent: &AgentInfo,
        attempt: usize,
        result: &AgentResult,
        accepted: bool,
        failure_reason: Option<String>,
    ) -> Result<AgentAttemptRecord> {
        let output_hash = sha256_hex(result.output.as_bytes());
        let duration_ms = result.duration.as_millis() as u64;
        let output_path = self.store.write_attempt_output(
            &self.state.session_id,
            ordinal,
            &agent.key,
            attempt,
            &result.output,
        )?;
        let output_path = relative_path(&self.store, &self.state.session_id, &output_path)?;
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::LlmAttemptCompleted {
                ordinal,
                agent_key: agent.key.clone(),
                attempt,
                output_hash: output_hash.clone(),
                tokens_in: result.tokens_in,
                tokens_out: result.tokens_out,
                duration_ms,
            },
        )?;
        if let Some(quality) = &result.quality {
            self.store.append_event(
                &self.state.session_id,
                PipelineEvent::QualityScored {
                    ordinal,
                    agent_key: agent.key.clone(),
                    attempt,
                    overall: quality.overall,
                    threshold: agent.quality_threshold,
                    accepted,
                },
            )?;
        }
        Ok(AgentAttemptRecord {
            attempt,
            output_path: Some(output_path),
            output_hash,
            tokens_in: result.tokens_in,
            tokens_out: result.tokens_out,
            duration_ms,
            quality: result.quality.clone(),
            accepted,
            failure_reason,
            created_at: Utc::now(),
        })
    }

    pub fn record_attempt_failed(
        &self,
        ordinal: usize,
        agent: &AgentInfo,
        attempt: usize,
        error: &str,
    ) -> Result<()> {
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::LlmAttemptFailed {
                ordinal,
                agent_key: agent.key.clone(),
                attempt,
                error: error.to_string(),
            },
        )
    }

    pub fn record_agent_retry(
        &self,
        ordinal: usize,
        agent: &AgentInfo,
        attempt: usize,
        reason: &str,
    ) -> Result<()> {
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::AgentRetried {
                ordinal,
                agent_key: agent.key.clone(),
                attempt,
                reason: reason.to_string(),
            },
        )
    }

    pub fn record_agent_completed(
        &mut self,
        ordinal: usize,
        agent: &AgentInfo,
        result: &AgentResult,
        attempts: Vec<AgentAttemptRecord>,
        prompt: PromptHashes,
    ) -> Result<()> {
        let output_path =
            self.store
                .write_output(&self.state.session_id, ordinal, &agent.key, &result.output)?;
        let output_path = relative_path(&self.store, &self.state.session_id, &output_path)?;
        let output_hash = sha256_hex(result.output.as_bytes());
        let record = AgentAuditRecord {
            ordinal,
            agent_key: agent.key.clone(),
            display_name: agent.display_name.clone(),
            phase: agent.phase,
            requested_model: agent.model.clone(),
            critical: agent.critical,
            quality_threshold: agent.quality_threshold,
            tool_access_level: agent.tool_access_level.clone(),
            prompt_record_path: prompt.prompt_record_path,
            prompt_hash: prompt.messages_hash,
            system_hash: prompt.system_hash,
            tools_hash: prompt.tools_hash,
            output_path,
            output_hash: output_hash.clone(),
            tokens_in: result.tokens_in,
            tokens_out: result.tokens_out,
            cost_usd: result.cost_usd,
            duration_ms: result.duration.as_millis() as u64,
            quality: result.quality.clone(),
            tool_use_log: result.tool_use_log.clone(),
            attempts,
            completed_at: Utc::now(),
        };
        self.store.write_agent(&self.state.session_id, &record)?;
        self.state.completed_agent_count = ordinal + 1;
        self.state.total_tokens_in += result.tokens_in;
        self.state.total_tokens_out += result.tokens_out;
        self.state.total_cost_usd += result.cost_usd;
        self.state.current_agent_key = None;
        self.state.updated_at = Utc::now();
        self.store.save_state(&self.state)?;
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::AgentCompleted {
                ordinal,
                agent_key: agent.key.clone(),
                output_hash,
            },
        )
    }

    pub fn complete(&mut self, final_output: &str) -> Result<()> {
        let final_output_hash = sha256_hex(final_output.as_bytes());
        self.state.status = BundleStatus::Completed;
        self.state.completed_at = Some(Utc::now());
        self.state.final_output_hash = Some(final_output_hash.clone());
        self.state.updated_at = Utc::now();
        self.store.save_state(&self.state)?;
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::RunCompleted {
                final_output_hash,
                completed_agent_count: self.state.completed_agent_count,
            },
        )
    }

    pub fn fail(&mut self, error: &str) -> Result<()> {
        self.state.status = BundleStatus::Failed;
        self.state.completed_at = Some(Utc::now());
        self.state.last_error = Some(error.to_string());
        self.state.updated_at = Utc::now();
        self.store.save_state(&self.state)?;
        self.store.append_event(
            &self.state.session_id,
            PipelineEvent::RunFailed {
                error: error.to_string(),
            },
        )
    }
}

#[derive(Clone, Debug)]
pub struct PromptHashes {
    pub prompt_record_path: String,
    pub messages_hash: String,
    pub system_hash: String,
    pub tools_hash: String,
}

fn relative_path(store: &PipelineBundleStore, session_id: &str, path: &Path) -> Result<String> {
    let base = store.bundle_dir(session_id);
    let rel: PathBuf = path
        .strip_prefix(&base)
        .with_context(|| format!("{} is outside {}", path.display(), base.display()))?
        .into();
    Ok(rel.to_string_lossy().replace('\\', "/"))
}
