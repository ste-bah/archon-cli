use super::*;

pub(super) struct RunIdentity {
    pub(super) manager_id: String,
    pub(super) cache_id: String,
}

pub(super) struct PreparedSubagentRun {
    pub(super) resolved_def: Option<CustomAgentDefinition>,
    pub(super) system_prompt: String,
    pub(super) model: String,
    pub(super) activity_agent_type: String,
    pub(super) max_turns: u32,
    pub(super) def_effort: Option<String>,
    pub(super) isolation: Option<String>,
}

impl AgentSubagentExecutor {
    pub(super) async fn register_subagent_run(
        &self,
        subagent_id: &str,
        request: &SubagentRequest,
    ) -> Result<RunIdentity, ExecutorError> {
        let manager_id = self
            .subagent_manager
            .lock()
            .await
            .register_with_id(subagent_id.to_string(), request.clone())
            .map_err(|e| ExecutorError::Internal(format!("Failed to register subagent: {e}")))?;
        if let Some(ref agent_type) = request.subagent_type {
            self.subagent_manager
                .lock()
                .await
                .register_name(agent_type.clone(), manager_id.clone());
        }
        tracing::info!(
            subagent_id = %manager_id,
            prompt_len = request.prompt.len(),
            "spawning one-shot subagent via AgentSubagentExecutor"
        );
        Ok(RunIdentity {
            manager_id,
            cache_id: subagent_id.to_string(),
        })
    }

    pub(super) async fn fire_subagent_start_hooks(
        &self,
        manager_id: &str,
        request: &SubagentRequest,
        nested: bool,
    ) {
        self.fire_hook(
            HookEvent::SubagentStart,
            serde_json::json!({
                "hook_event": "SubagentStart",
                "subagent_id": manager_id,
                "model": request.model,
                "prompt_length": request.prompt.len(),
            }),
        )
        .await;
        if nested {
            self.fire_hook(
                HookEvent::TaskCreated,
                serde_json::json!({
                    "hook_event": "TaskCreated",
                    "subagent_id": manager_id,
                }),
            )
            .await;
        }
    }

    pub(super) async fn prepare_subagent_run(
        &self,
        manager_id: &str,
        request: &SubagentRequest,
        ctx: &ToolContext,
    ) -> Result<PreparedSubagentRun, ExecutorError> {
        let resolved_def = self
            .resolve_agent_definition(manager_id, request, ctx)
            .await?;
        self.register_definition_hooks(request, resolved_def.as_ref());
        self.check_required_mcp_servers(manager_id, resolved_def.as_ref())
            .await?;
        let system_prompt = self.assemble_system_prompt(request, resolved_def.as_ref());
        let model = request
            .model
            .clone()
            .or_else(|| resolved_def.as_ref().and_then(|d| d.model.clone()))
            .unwrap_or_else(|| self.parent_model.clone());
        let activity_agent_type = resolved_def
            .as_ref()
            .map(|d| d.agent_type.as_str())
            .or(request.subagent_type.as_deref())
            .unwrap_or("general-purpose")
            .to_string();
        let def_effort = resolved_def.as_ref().and_then(|d| d.effort.clone());
        let isolation = request
            .isolation
            .clone()
            .or_else(|| resolved_def.as_ref().and_then(|d| d.isolation.clone()));

        Ok(PreparedSubagentRun {
            resolved_def,
            system_prompt,
            model,
            activity_agent_type,
            max_turns: request.max_turns,
            def_effort,
            isolation,
        })
    }

    async fn resolve_agent_definition(
        &self,
        manager_id: &str,
        request: &SubagentRequest,
        ctx: &ToolContext,
    ) -> Result<Option<CustomAgentDefinition>, ExecutorError> {
        if let Some(ref agent_type) = request.subagent_type {
            self.reject_nested_fork(manager_id, agent_type == "fork" && ctx.in_fork)
                .await?;
            return Ok(self.resolve_agent(agent_type));
        }
        if crate::agents::built_in::is_fork_enabled() {
            self.reject_nested_fork(manager_id, ctx.in_fork).await?;
            return Ok(self.resolve_agent("fork"));
        }
        Ok(None)
    }

    async fn reject_nested_fork(
        &self,
        manager_id: &str,
        should_reject: bool,
    ) -> Result<(), ExecutorError> {
        if !should_reject {
            return Ok(());
        }
        let reason = "Cannot fork inside a fork child".to_string();
        let _ = self
            .subagent_manager
            .lock()
            .await
            .mark_failed(manager_id, reason.clone());
        Err(ExecutorError::Internal(reason))
    }

    fn resolve_agent(&self, agent_type: &str) -> Option<CustomAgentDefinition> {
        let reg = self
            .agent_registry
            .read()
            .expect("agent registry lock poisoned");
        reg.resolve(agent_type).cloned()
    }

    fn register_definition_hooks(
        &self,
        request: &SubagentRequest,
        def: Option<&CustomAgentDefinition>,
    ) {
        let Some(def) = def else {
            return;
        };
        let Some(ref hooks_json) = def.hooks else {
            return;
        };
        match crate::agents::loader::parse_agent_hooks(hooks_json) {
            Ok(hook_pairs) => {
                if let Some(ref registry) = self.hook_registry {
                    for (event, config) in hook_pairs {
                        registry.register_session_hook(&self.session_id, event, config);
                    }
                    tracing::debug!(
                        agent_type = ?request.subagent_type,
                        "registered session-scoped hooks from agent definition"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    agent_type = ?request.subagent_type,
                    error = %e,
                    "failed to parse agent hooks"
                )
            }
        }
    }

    async fn check_required_mcp_servers(
        &self,
        manager_id: &str,
        def: Option<&CustomAgentDefinition>,
    ) -> Result<(), ExecutorError> {
        let Some(def) = def else {
            return Ok(());
        };
        let available_tools = self.tool_registry.tool_names();
        let available_mcp: Vec<String> = available_tools
            .iter()
            .filter(|n| n.starts_with("mcp__"))
            .map(|n| n.to_string())
            .collect();
        if def.has_required_mcp_servers(&available_mcp) {
            return Ok(());
        }
        let reason = format!(
            "Agent '{}' requires MCP servers {:?} but they are not available. Available MCP tools: {:?}",
            def.agent_type, def.required_mcp_servers, available_mcp,
        );
        let _ = self
            .subagent_manager
            .lock()
            .await
            .mark_failed(manager_id, reason.clone());
        Err(ExecutorError::Internal(reason))
    }

    fn assemble_system_prompt(
        &self,
        request: &SubagentRequest,
        def: Option<&CustomAgentDefinition>,
    ) -> String {
        let prompt = self.base_system_prompt(request, def);
        let prompt = self.with_fork_parent_context(prompt, def);
        let prompt = self.with_archon_md(prompt, def);
        let prompt = self.with_skills(prompt, def);
        let prompt = self.with_tool_guidance(prompt, def);
        let prompt = self.with_recalled_memory(prompt, def);
        let prompt = self.with_file_memory(prompt, def);
        self.with_leann_and_tags(prompt, def)
    }

    fn base_system_prompt(
        &self,
        request: &SubagentRequest,
        def: Option<&CustomAgentDefinition>,
    ) -> String {
        def.map(|d| d.system_prompt.clone()).unwrap_or_else(|| {
            request.subagent_type.as_ref()
                .map(|t| format!("You are a '{t}' subagent. Complete the task described in the user message. Be thorough and precise."))
                .unwrap_or_else(|| "You are a subagent. Complete the task described in the user message. Be thorough and precise.".into())
        })
    }

    fn with_fork_parent_context(
        &self,
        base: String,
        def: Option<&CustomAgentDefinition>,
    ) -> String {
        let is_fork = def.map(|d| d.agent_type == "fork").unwrap_or(false);
        if !is_fork {
            return base;
        }
        let parent_text: String = self
            .parent_system_prompt
            .iter()
            .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n\n");
        if parent_text.is_empty() {
            base
        } else {
            format!("<parent-context>\n{parent_text}\n</parent-context>\n\n{base}")
        }
    }

    fn with_archon_md(&self, prompt: String, def: Option<&CustomAgentDefinition>) -> String {
        if def.map(|d| d.omit_claude_md).unwrap_or(false) {
            return prompt;
        }
        let archon_md = crate::archonmd::load_hierarchical_archon_md(&self.working_dir);
        if archon_md.is_empty() {
            prompt
        } else {
            format!("{prompt}\n\n<archon-md>\n{archon_md}\n</archon-md>")
        }
    }

    fn with_skills(&self, prompt: String, def: Option<&CustomAgentDefinition>) -> String {
        let Some(skills) = def.and_then(|d| d.skills.as_ref()) else {
            return prompt;
        };
        if skills.is_empty() {
            return prompt;
        }
        let skills_list = skills.join(", ");
        format!(
            "{prompt}\n\n<available-skills>\nThe following skills are available to you: {skills_list}\nInvoke them by name when relevant to the task.\n</available-skills>"
        )
    }

    fn with_tool_guidance(&self, prompt: String, def: Option<&CustomAgentDefinition>) -> String {
        let Some(def) = def else {
            return prompt;
        };
        if def.tool_guidance.is_empty() {
            prompt
        } else {
            format!(
                "{prompt}\n\n<tool-guidance>\n{}\n</tool-guidance>",
                def.tool_guidance
            )
        }
    }

    fn with_recalled_memory(&self, prompt: String, def: Option<&CustomAgentDefinition>) -> String {
        let Some(def) = def else {
            return prompt;
        };
        if def.recall_queries.is_empty() {
            return prompt;
        }
        let Some(ref memory) = self.memory else {
            return prompt;
        };
        let memories = crate::agents::memory::load_agent_memory(
            &def.agent_type,
            &def.recall_queries,
            memory.as_ref(),
            def.memory_scope.as_ref(),
        );
        if memories.is_empty() {
            prompt
        } else {
            format!(
                "{prompt}\n\n<agent-memory>\n{}\n</agent-memory>",
                memories.join("\n---\n")
            )
        }
    }

    fn with_file_memory(&self, prompt: String, def: Option<&CustomAgentDefinition>) -> String {
        let Some(def) = def else {
            return prompt;
        };
        if let Some(memory_prompt) = crate::agents::memory::load_agent_memory_prompt(
            &def.agent_type,
            def.memory_scope.as_ref(),
            &self.working_dir,
        ) {
            format!("{prompt}\n\n{memory_prompt}")
        } else {
            prompt
        }
    }

    fn with_leann_and_tags(&self, prompt: String, def: Option<&CustomAgentDefinition>) -> String {
        let Some(def) = def else {
            return prompt;
        };
        let mut additions = Vec::new();
        if !def.leann_queries.is_empty() {
            let queries = def.leann_queries.join(", ");
            additions.push(format!("<leann-queries>\nRelevant code search queries for your task: {queries}\nUse these with the LEANN semantic search tool when exploring the codebase.\n</leann-queries>"));
        }
        if !def.tags.is_empty() {
            let tags = def.tags.join(", ");
            additions.push(format!("<agent-tags>\nYour memory tags: {tags}\nUse these tags when storing or recalling memories relevant to your role.\n</agent-tags>"));
        }
        if additions.is_empty() {
            prompt
        } else {
            format!("{prompt}\n\n{}", additions.join("\n\n"))
        }
    }
}
