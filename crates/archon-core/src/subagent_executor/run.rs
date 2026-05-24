use super::*;

impl AgentSubagentExecutor {
    pub(super) async fn run_subagent_to_completion(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        let manager_id = match self
            .subagent_manager
            .lock()
            .await
            .register_with_id(subagent_id.clone(), request.clone())
        {
            Ok(id) => id,
            Err(e) => {
                return Err(ExecutorError::Internal(format!(
                    "Failed to register subagent: {e}"
                )));
            }
        };
        let cache_id = subagent_id.clone();

        // Register agent name for SendMessage resolution (AGT-026).
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

        // --- TOP-OF-RUN HOOKS ----------------------------------------
        // Old H1 (SubagentStart) + H2 (TaskCreated if nested).
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
        if ctx.nested {
            self.fire_hook(
                HookEvent::TaskCreated,
                serde_json::json!({
                    "hook_event": "TaskCreated",
                    "subagent_id": manager_id,
                }),
            )
            .await;
        }

        // --- RESOLVE AGENT DEFINITION + FORK GUARDS ------------------
        // Predicate translated per mapping doc Section 1f:
        // explicit  fork → `request.subagent_type == Some("fork") && ctx.in_fork`
        // implicit  fork → `request.subagent_type.is_none() && is_fork_enabled() && ctx.in_fork`
        let resolved_def: Option<CustomAgentDefinition> =
            if let Some(ref agent_type) = request.subagent_type {
                if agent_type == "fork" && ctx.in_fork {
                    let _ = self
                        .subagent_manager
                        .lock()
                        .await
                        .mark_failed(&manager_id, "Cannot fork inside a fork child".into());
                    return Err(ExecutorError::Internal(
                        "Cannot fork inside a fork child".into(),
                    ));
                }
                let reg = self
                    .agent_registry
                    .read()
                    .expect("agent registry lock poisoned");
                reg.resolve(agent_type).cloned()
            } else if crate::agents::built_in::is_fork_enabled() {
                if ctx.in_fork {
                    let _ = self
                        .subagent_manager
                        .lock()
                        .await
                        .mark_failed(&manager_id, "Cannot fork inside a fork child".into());
                    return Err(ExecutorError::Internal(
                        "Cannot fork inside a fork child".into(),
                    ));
                }
                let reg = self
                    .agent_registry
                    .read()
                    .expect("agent registry lock poisoned");
                reg.resolve("fork").cloned()
            } else {
                None
            };

        // --- SYSTEM PROMPT ASSEMBLY ----------------------------------
        let base_system_prompt = resolved_def
            .as_ref()
            .map(|d| d.system_prompt.clone())
            .unwrap_or_else(|| {
                request.subagent_type.as_ref()
                    .map(|t| format!("You are a '{}' subagent. Complete the task described in the user message. Be thorough and precise.", t))
                    .unwrap_or_else(|| "You are a subagent. Complete the task described in the user message. Be thorough and precise.".into())
            });

        // Fork parent-context inheritance. Pass parent context through verbatim
        // and trust the configured LLM provider to handle context limits and
        // return any errors. Client-side truncation breaks the provider's
        // context coherence and is not our job.
        let is_fork = resolved_def
            .as_ref()
            .map(|d| d.agent_type == "fork")
            .unwrap_or(false);
        let system_prompt = if is_fork {
            let parent_text: String = self
                .parent_system_prompt
                .iter()
                .filter_map(|block| block.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n\n");
            if parent_text.is_empty() {
                base_system_prompt
            } else {
                format!(
                    "<parent-context>\n{parent_text}\n</parent-context>\n\n{base_system_prompt}"
                )
            }
        } else {
            base_system_prompt
        };

        // ARCHON.md prepend.
        let omit_claude_md = resolved_def
            .as_ref()
            .map(|d| d.omit_claude_md)
            .unwrap_or(false);
        let system_prompt = if !omit_claude_md {
            let archon_md = crate::archonmd::load_hierarchical_archon_md(&self.working_dir);
            if archon_md.is_empty() {
                system_prompt
            } else {
                format!("{system_prompt}\n\n<archon-md>\n{archon_md}\n</archon-md>")
            }
        } else {
            system_prompt
        };

        // Model fallback chain: request → def → parent.
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

        let max_turns = request.max_turns;

        let def_effort = resolved_def.as_ref().and_then(|d| d.effort.clone());

        let isolation = request
            .isolation
            .clone()
            .or_else(|| resolved_def.as_ref().and_then(|d| d.isolation.clone()));

        // Session-scoped hook registration from agent def.
        if let Some(ref def) = resolved_def
            && let Some(ref hooks_json) = def.hooks
        {
            match crate::agents::loader::parse_agent_hooks(hooks_json) {
                Ok(hook_pairs) => {
                    if let Some(ref registry) = self.hook_registry {
                        for (event, config) in hook_pairs {
                            registry.register_session_hook(&self.session_id, event, config);
                        }
                        tracing::debug!(agent_type = ?request.subagent_type, "registered session-scoped hooks from agent definition");
                    }
                }
                Err(e) => {
                    tracing::warn!(agent_type = ?request.subagent_type, error = %e, "failed to parse agent hooks")
                }
            }
        }

        // MCP pre-flight check.
        if let Some(ref def) = resolved_def {
            let available_tools = self.tool_registry.tool_names();
            let available_mcp: Vec<String> = available_tools
                .iter()
                .filter(|n| n.starts_with("mcp__"))
                .map(|n| n.to_string())
                .collect();
            if !def.has_required_mcp_servers(&available_mcp) {
                let reason = format!(
                    "Agent '{}' requires MCP servers {:?} but they are not available. Available MCP tools: {:?}",
                    def.agent_type, def.required_mcp_servers, available_mcp,
                );
                let _ = self
                    .subagent_manager
                    .lock()
                    .await
                    .mark_failed(&manager_id, reason.clone());
                return Err(ExecutorError::Internal(reason));
            }
        }

        // Skills injection.
        let system_prompt = if let Some(ref def) = resolved_def {
            if let Some(ref skills) = def.skills {
                if !skills.is_empty() {
                    let skills_list = skills.join(", ");
                    format!(
                        "{system_prompt}\n\n<available-skills>\nThe following skills are available to you: {skills_list}\nInvoke them by name when relevant to the task.\n</available-skills>"
                    )
                } else {
                    system_prompt
                }
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // Tool guidance injection.
        let system_prompt = if let Some(ref def) = resolved_def {
            if !def.tool_guidance.is_empty() {
                format!(
                    "{system_prompt}\n\n<tool-guidance>\n{}\n</tool-guidance>",
                    def.tool_guidance
                )
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // Agent memory (recall_queries) injection.
        let system_prompt = if let Some(ref def) = resolved_def {
            if !def.recall_queries.is_empty() {
                if let Some(ref memory) = self.memory {
                    let memories = crate::agents::memory::load_agent_memory(
                        &def.agent_type,
                        &def.recall_queries,
                        memory.as_ref(),
                        def.memory_scope.as_ref(),
                    );
                    if !memories.is_empty() {
                        let mem_block = memories.join("\n---\n");
                        format!("{system_prompt}\n\n<agent-memory>\n{mem_block}\n</agent-memory>")
                    } else {
                        system_prompt
                    }
                } else {
                    system_prompt
                }
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // File-based memory prompt injection.
        let system_prompt = if let Some(ref def) = resolved_def {
            if let Some(memory_prompt) = crate::agents::memory::load_agent_memory_prompt(
                &def.agent_type,
                def.memory_scope.as_ref(),
                &self.working_dir,
            ) {
                format!("{system_prompt}\n\n{memory_prompt}")
            } else {
                system_prompt
            }
        } else {
            system_prompt
        };

        // LEANN queries + tags injection.
        let system_prompt = if let Some(ref def) = resolved_def {
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
                system_prompt
            } else {
                format!("{system_prompt}\n\n{}", additions.join("\n\n"))
            }
        } else {
            system_prompt
        };

        // Build the subagent tool registry.
        let (tool_defs, tool_reg) = self
            .build_subagent_tools(&request, resolved_def.as_ref())
            .await;

        let requested_cwd = super::paths::resolve_cwd(&self.working_dir, request.cwd.as_deref());

        let worktree_info = if isolation.as_deref() == Some("worktree") {
            let source_root = requested_cwd.as_deref().unwrap_or(&self.working_dir);
            match super::paths::create_worktree(source_root, &manager_id) {
                Ok(info) => {
                    tracing::info!(subagent_id = %manager_id, worktree = %info.worktree_path.display(), "created worktree for isolated subagent");
                    Some(info)
                }
                Err(e) => {
                    let _ = self
                        .subagent_manager
                        .lock()
                        .await
                        .mark_failed(&manager_id, e.clone());
                    return Err(ExecutorError::Internal(e));
                }
            }
        } else {
            None
        };

        // Cache worktree info for on_visible_complete.
        if let Some(ref wt) = worktree_info {
            self.worktree_cache
                .lock()
                .await
                .insert(cache_id.clone(), wt.clone());
        }
        // Cache memory meta for on_inner_complete.
        self.memory_cache.lock().await.insert(
            cache_id.clone(),
            MemoryMeta {
                agent_type: resolved_def.as_ref().map(|d| d.agent_type.clone()),
                memory_scope: resolved_def.as_ref().and_then(|d| d.memory_scope.clone()),
                tags: resolved_def
                    .as_ref()
                    .map(|d| d.tags.clone())
                    .unwrap_or_default(),
            },
        );

        let working_dir = worktree_info
            .as_ref()
            .map(|wt| wt.worktree_path.clone())
            .or_else(|| requested_cwd.clone())
            .unwrap_or_else(|| self.working_dir.clone());

        let subagent_mode = {
            let parent_mode = self.parent_permission_mode.lock().await.clone();
            let requested_mode = resolved_def
                .as_ref()
                .and_then(|definition| definition.permission_mode.as_ref());
            crate::agents::permissions_overlay::resolve_subagent_agent_mode(
                &parent_mode,
                requested_mode,
                parent_mode == "bypassPermissions",
            )
        };

        // Build the subagent's own ToolContext. in_fork inherits from
        // the caller OR is set when this subagent is itself a fork.
        let subagent_in_fork = ctx.in_fork
            || resolved_def
                .as_ref()
                .map(|d| d.agent_type == "fork")
                .unwrap_or(false);
        let tool_ctx = ToolContext {
            working_dir,
            session_id: self.session_id.clone(),
            mode: subagent_mode,
            extra_dirs: vec![],
            in_fork: subagent_in_fork,
            nested: false,
            // TASK-AGS-107: propagate cancel from parent context so child
            // subagents inherit the cancellation chain.
            cancel_parent: ctx.cancel_parent.clone(),
            // GHOST-006: propagate sandbox backend to subagent tool calls.
            sandbox: ctx.sandbox.clone(),
            activity_sink: ctx.activity_sink.clone(),
        };

        let mut runner = crate::subagent::runner::SubagentRunner::new(
            self.client.clone(),
            system_prompt,
            tool_defs,
            Arc::new(tool_reg),
            tool_ctx,
            model.clone(),
            max_turns,
            request.timeout_secs,
            Arc::clone(&self.agent_config),
            Arc::clone(&self.identity),
        );

        if let Some(effort) = def_effort {
            runner.set_effort(effort);
        }
        runner.set_activity_actor(cache_id.clone(), activity_agent_type.clone());
        let activity_model = runner.model().to_string();

        if let Some(ref def) = resolved_def
            && let Some(ref reminder) = def.critical_system_reminder
        {
            runner.set_critical_system_reminder(reminder.clone());
        }

        // Transcript recording (AGT-024).
        if let Some(store) = crate::agents::transcript::AgentTranscriptStore::new(&self.session_id)
        {
            let meta = crate::agents::transcript::AgentMetadata {
                agent_type: resolved_def
                    .as_ref()
                    .map(|d| d.agent_type.clone())
                    .unwrap_or_else(|| "general-purpose".into()),
                worktree_path: worktree_info
                    .as_ref()
                    .map(|wt| wt.worktree_path.display().to_string()),
                description: Some(request.prompt.chars().take(200).collect()),
                filename: resolved_def.as_ref().and_then(|d| d.filename.clone()),
            };
            store.write_metadata(&manager_id, &meta);
            runner.set_transcript(store, manager_id.clone());
        }

        // Inject resume messages if pending (from SendMessage resume).
        if let Some(resume_msgs) = self.pending_resume_messages.lock().await.take() {
            tracing::info!(
                count = resume_msgs.len(),
                "Injecting resume messages into SubagentRunner"
            );
            runner.set_initial_messages(resume_msgs);
        }

        runner.set_pending_message_source(Arc::clone(&self.subagent_manager), manager_id.clone());

        {
            let mgr = self.subagent_manager.lock().await;
            if let Some(flag) = mgr.get_shutdown_flag(&manager_id) {
                runner.set_shutdown_flag(flag);
            }
            if let Some(tracker) = mgr.get_progress_tracker_arc(&manager_id) {
                runner.set_progress_tracker(tracker);
            }
        }

        self.emit_subagent_started(&cache_id, &activity_agent_type, &activity_model);

        // --- RUN THE RUNNER ------------------------------------------
        let runner_result = runner.run(&request.prompt).await;

        // Convert to a Result<String, String> and fire inner-complete
        // UNCONDITIONALLY (PRESERVE-D8).
        let inner_result: Result<String, String> = match runner_result {
            Ok(text) => Ok(text),
            Err(e) => Err(format!("Subagent failed: {e}")),
        };
        self.emit_subagent_finished(
            &cache_id,
            &activity_agent_type,
            &activity_model,
            &inner_result,
        );
        self.on_inner_complete(cache_id.clone(), inner_result.clone())
            .await;

        match inner_result {
            Ok(text) => Ok(text),
            Err(err) => Err(ExecutorError::Internal(err)),
        }
    }
}
