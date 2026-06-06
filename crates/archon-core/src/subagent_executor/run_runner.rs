use std::sync::Arc;

use super::run_prepare::{PreparedSubagentRun, RunIdentity};
use super::*;

impl AgentSubagentExecutor {
    pub(super) async fn build_subagent_runner(
        &self,
        ids: &RunIdentity,
        request: &SubagentRequest,
        ctx: &ToolContext,
        prepared: &PreparedSubagentRun,
    ) -> Result<crate::subagent::runner::SubagentRunner, ExecutorError> {
        let (tool_defs, tool_reg) = self
            .build_subagent_tools(request, prepared.resolved_def.as_ref())
            .await;
        let requested_cwd = super::paths::resolve_cwd(&self.working_dir, request.cwd.as_deref());
        let worktree_info = self
            .create_run_worktree(&ids.manager_id, requested_cwd.as_deref(), prepared)
            .await?;
        self.cache_run_metadata(&ids.cache_id, &worktree_info, prepared)
            .await;
        let tool_ctx = self
            .build_child_tool_context(ctx, requested_cwd, worktree_info.as_ref(), prepared)
            .await;
        let mut runner = crate::subagent::runner::SubagentRunner::new(
            self.client.clone(),
            prepared.system_prompt.clone(),
            tool_defs,
            Arc::new(tool_reg),
            tool_ctx,
            prepared.model.clone(),
            prepared.max_turns,
            request.timeout_secs,
            Arc::clone(&self.agent_config),
            Arc::clone(&self.identity),
        );
        self.configure_runner(&mut runner, ids, request, worktree_info.as_ref(), prepared)
            .await;
        Ok(runner)
    }

    async fn create_run_worktree(
        &self,
        manager_id: &str,
        requested_cwd: Option<&std::path::Path>,
        prepared: &PreparedSubagentRun,
    ) -> Result<Option<WorktreeInfo>, ExecutorError> {
        if prepared.isolation.as_deref() != Some("worktree") {
            return Ok(None);
        }
        let source_root = requested_cwd.unwrap_or(&self.working_dir);
        match super::paths::create_worktree(source_root, manager_id) {
            Ok(info) => {
                tracing::info!(
                    subagent_id = %manager_id,
                    worktree = %info.worktree_path.display(),
                    "created worktree for isolated subagent"
                );
                Ok(Some(info))
            }
            Err(e) => {
                let _ = self
                    .subagent_manager
                    .lock()
                    .await
                    .mark_failed(manager_id, e.clone());
                Err(ExecutorError::Internal(e))
            }
        }
    }

    async fn cache_run_metadata(
        &self,
        cache_id: &str,
        worktree_info: &Option<WorktreeInfo>,
        prepared: &PreparedSubagentRun,
    ) {
        if let Some(wt) = worktree_info {
            self.worktree_cache
                .lock()
                .await
                .insert(cache_id.to_string(), wt.clone());
        }
        self.memory_cache.lock().await.insert(
            cache_id.to_string(),
            MemoryMeta {
                agent_type: prepared.resolved_def.as_ref().map(|d| d.agent_type.clone()),
                memory_scope: prepared
                    .resolved_def
                    .as_ref()
                    .and_then(|d| d.memory_scope.clone()),
                tags: prepared
                    .resolved_def
                    .as_ref()
                    .map(|d| d.tags.clone())
                    .unwrap_or_default(),
            },
        );
    }

    async fn build_child_tool_context(
        &self,
        parent_ctx: &ToolContext,
        requested_cwd: Option<std::path::PathBuf>,
        worktree_info: Option<&WorktreeInfo>,
        prepared: &PreparedSubagentRun,
    ) -> ToolContext {
        let working_dir = worktree_info
            .map(|wt| wt.worktree_path.clone())
            .or(requested_cwd)
            .unwrap_or_else(|| self.working_dir.clone());
        let parent_mode = self.parent_permission_mode.lock().await.clone();
        let requested_mode = prepared
            .resolved_def
            .as_ref()
            .and_then(|definition| definition.permission_mode.as_ref());
        let mode = crate::agents::permissions_overlay::resolve_subagent_agent_mode(
            &parent_mode,
            requested_mode,
            parent_mode == "bypassPermissions",
        );
        let in_fork = parent_ctx.in_fork
            || prepared
                .resolved_def
                .as_ref()
                .map(|d| d.agent_type == "fork")
                .unwrap_or(false);
        let extra_dirs = child_extra_dirs(parent_ctx, &working_dir);
        ToolContext {
            working_dir,
            session_id: self.session_id.clone(),
            mode,
            extra_dirs,
            in_fork,
            nested: false,
            cancel_parent: parent_ctx.cancel_parent.clone(),
            sandbox: parent_ctx.sandbox.clone(),
            activity_sink: parent_ctx.activity_sink.clone(),
        }
    }

    async fn configure_runner(
        &self,
        runner: &mut crate::subagent::runner::SubagentRunner,
        ids: &RunIdentity,
        request: &SubagentRequest,
        worktree_info: Option<&WorktreeInfo>,
        prepared: &PreparedSubagentRun,
    ) {
        if let Some(effort) = prepared.def_effort.clone() {
            runner.set_effort(effort);
        }
        runner.set_activity_actor(ids.cache_id.clone(), prepared.activity_agent_type.clone());
        if let Some(ref def) = prepared.resolved_def
            && let Some(ref reminder) = def.critical_system_reminder
        {
            runner.set_critical_system_reminder(reminder.clone());
        }
        self.configure_transcript(runner, &ids.manager_id, request, worktree_info, prepared);
        self.configure_resume_and_progress(runner, &ids.manager_id)
            .await;
    }

    fn configure_transcript(
        &self,
        runner: &mut crate::subagent::runner::SubagentRunner,
        manager_id: &str,
        request: &SubagentRequest,
        worktree_info: Option<&WorktreeInfo>,
        prepared: &PreparedSubagentRun,
    ) {
        let Some(store) = crate::agents::transcript::AgentTranscriptStore::new(&self.session_id)
        else {
            return;
        };
        let meta = crate::agents::transcript::AgentMetadata {
            agent_type: prepared
                .resolved_def
                .as_ref()
                .map(|d| d.agent_type.clone())
                .unwrap_or_else(|| "general-purpose".into()),
            worktree_path: worktree_info.map(|wt| wt.worktree_path.display().to_string()),
            description: Some(request.prompt.chars().take(200).collect()),
            filename: prepared
                .resolved_def
                .as_ref()
                .and_then(|d| d.filename.clone()),
        };
        store.write_metadata(manager_id, &meta);
        runner.set_transcript(store, manager_id.to_string());
    }

    async fn configure_resume_and_progress(
        &self,
        runner: &mut crate::subagent::runner::SubagentRunner,
        manager_id: &str,
    ) {
        if let Some(resume_msgs) = self.pending_resume_messages.lock().await.take() {
            tracing::info!(
                count = resume_msgs.len(),
                "Injecting resume messages into SubagentRunner"
            );
            runner.set_initial_messages(resume_msgs);
        }
        runner
            .set_pending_message_source(Arc::clone(&self.subagent_manager), manager_id.to_string());
        let mgr = self.subagent_manager.lock().await;
        if let Some(flag) = mgr.get_shutdown_flag(manager_id) {
            runner.set_shutdown_flag(flag);
        }
        if let Some(tracker) = mgr.get_progress_tracker_arc(manager_id) {
            runner.set_progress_tracker(tracker);
        }
    }
}

fn child_extra_dirs(
    parent_ctx: &ToolContext,
    child_working_dir: &std::path::Path,
) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if !parent_ctx.working_dir.as_os_str().is_empty()
        && parent_ctx.working_dir.as_path() != child_working_dir
    {
        dirs.push(parent_ctx.working_dir.clone());
    }
    for extra_dir in &parent_ctx.extra_dirs {
        let resolved = if extra_dir.is_absolute() {
            extra_dir.clone()
        } else {
            parent_ctx.working_dir.join(extra_dir)
        };
        if !dirs.contains(&resolved) && resolved.as_path() != child_working_dir {
            dirs.push(resolved);
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::child_extra_dirs;
    use archon_tools::tool::ToolContext;
    use std::path::{Path, PathBuf};

    #[test]
    fn child_extra_dirs_preserve_parent_project_when_cwd_changes() {
        let parent = ToolContext {
            working_dir: PathBuf::from("/project-1"),
            extra_dirs: vec![PathBuf::from("assets"), PathBuf::from("/shared")],
            ..ToolContext::default()
        };

        let dirs = child_extra_dirs(&parent, Path::new("/repo"));

        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/project-1"),
                PathBuf::from("/project-1/assets"),
                PathBuf::from("/shared"),
            ]
        );
    }

    #[test]
    fn child_extra_dirs_do_not_duplicate_child_working_dir() {
        let parent = ToolContext {
            working_dir: PathBuf::from("/repo"),
            extra_dirs: vec![PathBuf::from("/repo")],
            ..ToolContext::default()
        };

        let dirs = child_extra_dirs(&parent, Path::new("/repo"));

        assert!(dirs.is_empty());
    }
}
