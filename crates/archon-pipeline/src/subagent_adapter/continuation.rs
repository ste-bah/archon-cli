//! Completed sessions are resumed only by an explicit continuation dispatch.
use super::*;
use archon_tools::subagent_session::CompletedHistory;
use archon_tools::workflow_read_guard::WorkflowReadGuard;
use std::collections::HashMap;
use std::sync::Mutex;

type ReadGuard = Option<Arc<WorkflowReadGuard>>;
pub(super) type SessionCache = Arc<Mutex<HashMap<String, SessionState>>>;

pub(super) enum SessionState {
    Running(String),
    Complete(SessionLeaseData),
}

pub(super) struct SessionLeaseData {
    id: String,
    request: AgentExecutionRequest,
    history: CompletedHistory,
    read_guard: ReadGuard,
}

struct SessionLease {
    cache: SessionCache,
    key: String,
    id: String,
    request: AgentExecutionRequest,
    history: CompletedHistory,
    read_guard: ReadGuard,
}

fn session_key(request: &AgentExecutionRequest) -> Result<String> {
    Ok(serde_json::to_string(&(
        &request.session_id,
        request.ordinal,
        &request.agent.key,
    ))?)
}

fn policy(request: &AgentExecutionRequest) -> String {
    // Feedback may change messages, nothing else may change the agent's authority.
    format!(
        "{:?}",
        (
            &request.pipeline_type,
            &request.task,
            &request.cwd,
            request.attempt,
            &request.agent,
            &request.system,
            &request.tools,
            &request.allowed_tools,
            &request.write_roots,
            request.disable_auto_background
        )
    )
}

/// The guard a fresh workflow session runs under. A call that can mutate
/// files gets the write-first read budget; a call that can only inspect and
/// run Bash gets the shell admissions alone (Issue-21: a verifier spent 25
/// minutes in `cargo build --release` in the canonical checkout, which the
/// guard refuses for coders, because no guard was installed for it). A call
/// with neither has nothing to admit.
fn workflow_guard(
    client: &SubagentPipelineClient,
    request: &AgentExecutionRequest,
) -> ReadGuard {
    let tools = SubagentPipelineClient::allowed_tools(request);
    let write_capable = tools.iter().any(|name| {
        matches!(
            name.as_str(),
            "Write" | "Edit" | "ApplyPatch" | "NotebookEdit" | "MultiEdit"
        )
    });
    let settings = &client.workflow_read_guard;
    if write_capable {
        Some(Arc::new(WorkflowReadGuard::from_settings(settings)))
    } else if tools.iter().any(|name| name == "Bash") {
        Some(Arc::new(WorkflowReadGuard::shell_only(settings)))
    } else {
        None
    }
}

impl SessionLease {
    fn begin(
        client: &SubagentPipelineClient,
        request: &AgentExecutionRequest,
        continuing: bool,
    ) -> Result<Self> {
        let key = session_key(request)?;
        let mut sessions = client
            .sessions
            .lock()
            .map_err(|_| anyhow!("session cache poisoned"))?;
        if matches!(sessions.get(&key), Some(SessionState::Running(_))) {
            anyhow::bail!("agent session is already running; concurrent continuation refused");
        }
        let data = if continuing {
            let Some(SessionState::Complete(previous)) = sessions.get(&key) else {
                anyhow::bail!("no completed agent session for validation repair");
            };
            if policy(&previous.request) != policy(request) {
                anyhow::bail!("validation repair changed agent identity or execution policy");
            }
            let Some(SessionState::Complete(data)) = sessions.remove(&key) else {
                unreachable!()
            };
            data
        } else {
            let read_guard = (request.pipeline_type == PipelineType::Workflow)
                .then(|| workflow_guard(client, request))
                .flatten();
            SessionLeaseData {
                id: format!(
                    "{}-{}-{}-{}",
                    request.session_id,
                    request.ordinal,
                    request.agent.key,
                    uuid::Uuid::new_v4()
                ),
                request: request.clone(),
                history: CompletedHistory::default(),
                read_guard,
            }
        };
        sessions.insert(key.clone(), SessionState::Running(data.id.clone()));
        Ok(Self {
            cache: client.sessions.clone(),
            key,
            id: data.id,
            request: data.request,
            history: data.history,
            read_guard: data.read_guard,
        })
    }

    fn complete(&mut self) -> Result<()> {
        self.cache
            .lock()
            .map_err(|_| anyhow!("session cache poisoned"))?
            .insert(
                self.key.clone(),
                SessionState::Complete(SessionLeaseData {
                    id: self.id.clone(),
                    request: self.request.clone(),
                    history: self.history.clone(),
                    read_guard: self.read_guard.clone(),
                }),
            );
        Ok(())
    }
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        if let Ok(mut cache) = self.cache.lock() {
            if matches!(cache.get(&self.key), Some(SessionState::Running(id)) if id == &self.id) {
                cache.remove(&self.key);
            }
        }
    }
}

impl SubagentPipelineClient {
    pub(super) async fn execute_session(
        &self,
        request: AgentExecutionRequest,
        continuing: bool,
    ) -> Result<LlmResponse> {
        let mut lease = SessionLease::begin(self, &request, continuing)?;
        // The first invocation owns execution policy, credentials and timeouts.
        let request = if continuing {
            let mut original = lease.request.clone();
            original.messages = request.messages;
            original
        } else {
            request
        };
        let prompt = if continuing {
            SubagentPipelinePrompt {
                prompt: values_to_text(&request.messages),
                system: lease.request.system.clone(),
            }
        } else {
            Self::prompt_for_request(&request)
        };
        let activity_model = self.activity_model(&request.agent.model);
        let allowed_tools = Self::allowed_tools(&request);
        let strict_workspace_boundary = Self::strict_workspace_boundary(&request, &allowed_tools);
        let provider_env = workflow_provider_env_source(&request);
        let system = prompt.system;
        let req = SubagentRequest {
            prompt: prompt.prompt,
            model: Some(activity_model),
            allowed_tools,
            max_turns: SubagentRequest::DEFAULT_MAX_TURNS,
            timeout_secs: request
                .timeout_secs
                .unwrap_or(SubagentRequest::DEFAULT_TIMEOUT_SECS),
            subagent_type: Some(request.agent.key.clone()),
            run_in_background: false,
            cwd: Some(self.cwd_for_request(&request)),
            isolation: strict_workspace_boundary.then(|| "workspace-boundary".to_string()),
            // The agent's workspace PLUS the artifact roots its task declared.
            // Those roots routinely sit outside the repository — one reference
            // PRD's whole purpose is a registry under a project directory that
            // is not a git repository at all — which is why the set comes from
            // the host's own artifact resolution rather than from the workspace
            // alone. See `declared_write_roots` for the three conditions.
            write_roots: self.declared_write_roots(&request),
            provider_env,
        };

        let cancel = self
            .context
            .cancel_parent
            .as_ref()
            .map(|token| token.child_token())
            .unwrap_or_default();
        // An outer attempt deadline can drop this future while its executor is
        // spawned. Propagate cancellation instead of leaving that agent running.
        let _cancel_on_drop = cancel.clone().drop_guard();
        let mut tool_context = self.context.clone();
        tool_context.cancel_parent = Some(cancel.clone());
        tool_context.workflow_read_guard = lease.read_guard.clone();
        tool_context.audit_landing = archon_tools::audit_landing::current();
        tool_context
            .denied_directory_names
            .extend(archon_tools::read_boundary::current());

        let subagent_id = lease.id.clone();
        let mut run: std::pin::Pin<Box<dyn std::future::Future<Output = SubagentOutcome> + Send>> =
            if request.disable_auto_background {
                Box::pin(run_subagent_foreground_with_system(
                    subagent_id,
                    req,
                    system,
                    cancel.clone(),
                    tool_context,
                ))
            } else {
                Box::pin(run_subagent_with_system(
                    subagent_id,
                    req,
                    system,
                    cancel.clone(),
                    tool_context,
                ))
            };
        // An exact host policy carries an explicit timeout decision. None is
        // unlimited here, not omission that restores the runner's default.
        if request.pipeline_type == PipelineType::Workflow
            && request
                .allowed_tools
                .iter()
                .any(|tool| tool == EXACT_TOOL_POLICY_MARKER)
        {
            let limit = request
                .timeout_secs
                .map(archon_tools::host_timeout::HostTimeout::Finite)
                .unwrap_or(archon_tools::host_timeout::HostTimeout::Unlimited);
            run = Box::pin(archon_tools::host_timeout::scope(limit, run));
        }
        run = Box::pin(archon_tools::subagent_session::scope(
            archon_tools::subagent_session::SubagentSession {
                agent_id: lease.id.clone(),
                history: lease.history.clone(),
                continuing,
            },
            run,
        ));
        let mut timed_out = false;
        let outcome = if let Some(timeout_secs) = request.timeout_secs {
            let timeout = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs.max(1)));
            tokio::pin!(timeout);
            tokio::select! {
                outcome = &mut run => outcome,
                _ = &mut timeout => {
                    timed_out = true;
                    cancel.cancel();
                    run.await
                }
            }
        } else {
            run.await
        };

        let response = llm_response_for_subagent_outcome(outcome, timed_out, request.timeout_secs)?;
        lease.complete()?;
        Ok(response)
    }
}

#[cfg(test)]
#[path = "continuation_tests.rs"]
mod tests;
