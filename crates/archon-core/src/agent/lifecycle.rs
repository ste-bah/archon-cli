use super::*;

impl Agent {
    pub fn new(
        client: Arc<dyn LlmProvider>,
        registry: ToolRegistry,
        config: AgentConfig,
        event_tx: tokio::sync::mpsc::UnboundedSender<TimestampedEvent>,
        agent_registry: Arc<std::sync::RwLock<AgentRegistry>>,
    ) -> Self {
        let permission_store: Arc<dyn crate::hooks::PermissionStore> =
            Arc::new(crate::hooks::RuntimePermissionStore::new(
                dirs::home_dir()
                    .unwrap_or_default()
                    .join(".archon")
                    .join("settings.json"),
                config.working_dir.join(".archon").join("settings.json"),
            ));
        Self {
            client,
            registry,
            config,
            state: ConversationState::default(),
            event_tx,
            checkpoint_store: None,
            plan_store: None,
            turn_number: 0,
            memory: None,
            memory_injector: MemoryInjector::new(),
            extraction_config: ExtractionConfig::default(),
            extraction_state: ExtractionState::default(),
            auto_extractor: None,
            auto_extraction_tasks: Vec::new(),
            auto_evaluator: None,
            subagent_manager: Arc::new(Mutex::new(SubagentManager::default())),
            show_thinking: Arc::new(AtomicBool::new(true)),
            session_stats: Arc::new(Mutex::new(SessionStats::default())),
            hook_registry: None,
            file_watch_manager: Arc::new(crate::hooks::FileWatchManager::new(100)),
            permission_response_rx: None,
            inner_voice: None,
            ask_user_response_rx: None,
            previous_permission_mode: None,
            denial_log: Arc::new(Mutex::new(archon_permissions::denial_log::DenialLog::new())),
            agent_registry,
            personality_briefing: None,
            memory_briefing: None,
            permission_store,
            critical_system_reminder: None,
            pending_resume_messages: Arc::new(tokio::sync::Mutex::new(None)),
            metrics: None,
            // Reference: archon-pipeline/src/learning/gnn/auto_trainer_runtime.rs.
            // Wired by the binary at startup via set_record_memory_callback /
            // set_record_correction_callback so the AutoTrainer's record_*
            // methods get called from agent's memory + correction code paths.
            record_memory_callback: None,
            record_correction_callback: None,
            record_user_correction_event_callback: None,
            record_reasoning_turn_callback: None,
            reasoning_evidence_refs: Vec::new(),
            // TASK #245: wired by the binary at startup; default None makes
            // tests and non-interactive paths no-op.
            inner_voice_change_callback: None,
        }
    }

    /// TASK-AGS-105: install the `AgentSubagentExecutor` into the process
    /// OnceLock so `AgentTool::execute` and `TaskCreateTool::execute` can
    /// resolve it via `archon_tools::subagent_executor::get_subagent_executor`.
    ///
    /// Called explicitly by the embedder (CLI, tests) AFTER constructing the
    /// `Agent` with its full field set (hook_registry, memory, etc.). This is
    /// a separate step from `Agent::new` because many of the fields the
    /// TASK-AGS-107: set the cancel token for Ctrl+C propagation.
    /// Called from the input handler spawn in main.rs before
    /// process_message, cleared afterward.
    pub fn set_cancel_token(&mut self, token: Option<tokio_util::sync::CancellationToken>) {
        self.config.cancel_token = token;
    }

    /// executor needs are set via post-construction setters
    /// (`set_hook_registry`, `set_memory`, ...). The install is idempotent
    /// per-process (OnceLock semantics): first caller wins.
    pub fn install_subagent_executor(&self) {
        let exec = crate::subagent_executor::AgentSubagentExecutor::new(
            Arc::clone(&self.client),
            self.registry.clone(),
            Arc::clone(&self.subagent_manager),
            Arc::clone(&self.agent_registry),
            self.hook_registry.as_ref().map(Arc::clone),
            self.memory.as_ref().map(Arc::clone),
            self.config.working_dir.clone(),
            self.config.session_id.clone(),
            self.config.model.clone(),
            self.config.system_prompt.clone(),
            Arc::clone(&self.config.permission_mode),
            Arc::clone(&self.pending_resume_messages),
            Arc::new(self.config.clone()),
            Arc::new(self.identity_provider().cloned().unwrap_or_else(|| {
                archon_llm::identity::IdentityProvider::new(
                    archon_llm::identity::IdentityMode::Clean,
                    self.config.session_id.clone(),
                    String::new(),
                    String::new(),
                )
            })),
        );
        archon_tools::subagent_executor::install_subagent_executor(Arc::new(exec));
    }

    /// Enable the inner voice feature. The supplied state is shared so that
    /// external components (slash commands, compaction handlers) can inspect
    /// or snapshot it.
    /// Set the personality briefing text (injected on first turn only).
    pub fn set_personality_briefing(&mut self, text: String) {
        self.personality_briefing = Some(text);
    }

    /// Set the memory garden briefing text (injected on first turn only).
    pub fn set_memory_briefing(&mut self, text: String) {
        self.memory_briefing = Some(text);
    }

    /// Set the critical system reminder (re-injected every turn, AGT-022).
    pub fn set_critical_system_reminder(&mut self, text: String) {
        if text.is_empty() {
            self.critical_system_reminder = None;
        } else {
            self.critical_system_reminder = Some(text);
        }
    }

    pub fn set_inner_voice(&mut self, iv: Arc<Mutex<InnerVoice>>) {
        self.inner_voice = Some(iv);
    }

    pub fn set_channel_metrics(&mut self, metrics: Arc<dyn ChannelMetricSink>) {
        self.metrics = Some(metrics);
    }

    /// Access the inner voice handle, if enabled.
    pub fn inner_voice(&self) -> Option<&Arc<Mutex<InnerVoice>>> {
        self.inner_voice.as_ref()
    }

    /// Access the subagent manager (read-only) for status queries.
    pub fn subagent_manager(&self) -> Arc<Mutex<SubagentManager>> {
        Arc::clone(&self.subagent_manager)
    }

    /// Close the event channel so receivers know the agent is done.
    /// Used by print mode to unblock the event consumer task.
    pub fn close_event_channel(&mut self) {
        // Replace the sender with a closed one by dropping it.
        // TASK-AGS-102: unbounded variant — same drop-to-close semantics.
        let (tx, _) = tokio::sync::mpsc::unbounded_channel();
        self.event_tx = tx;
        // The old sender is dropped, closing the channel
    }

    /// Set the hook registry for pre/post tool execution hooks.
    pub fn set_hook_registry(&mut self, registry: Arc<crate::hooks::HookRegistry>) {
        self.hook_registry = Some(registry);
    }

    /// Add dynamic watch paths from hooks (REQ-HOOK-017).
    pub fn add_watch_paths(&self, paths: Vec<String>) {
        self.file_watch_manager.add_watch_paths(paths);
    }

    /// Clear all dynamic watch paths (called on SessionEnd).
    pub fn clear_watch_paths(&self) {
        self.file_watch_manager.clear();
    }

    /// Fire a hook by event with a JSON payload. Returns the aggregated result.
    /// No-op (returns empty aggregate) if no registry is set.
    pub async fn fire_hook(
        &self,
        event: crate::hooks::HookEvent,
        payload: serde_json::Value,
    ) -> crate::hooks::AggregatedHookResult {
        if let Some(ref registry) = self.hook_registry {
            registry
                .execute_hooks(
                    event,
                    payload,
                    &self.config.working_dir,
                    &self.config.session_id,
                )
                .await
        } else {
            crate::hooks::AggregatedHookResult::new()
        }
    }

    /// Fire a hook without holding `&self` across the returned future.
    ///
    /// Clones `hook_registry` (`Arc<HookRegistry>`) and the required
    /// config fields up-front so the returned future is
    /// `Send + 'static` and can outlive a `MutexGuard<Agent>`. Call
    /// sites drop the guard before `.await`ing the future, avoiding
    /// the non-`Send` compile error when `tokio::spawn`ing a block
    /// that locks an `Arc<Mutex<Agent>>` and then awaits hook work.
    ///
    /// Semantically identical to [`Agent::fire_hook`].
    pub fn fire_hook_detached(
        &self,
        event: crate::hooks::HookEvent,
        payload: serde_json::Value,
    ) -> impl std::future::Future<Output = crate::hooks::AggregatedHookResult> + Send + 'static
    {
        let registry = self.hook_registry.clone();
        let working_dir = self.config.working_dir.clone();
        let session_id = self.config.session_id.clone();
        async move {
            if let Some(registry) = registry {
                registry
                    .execute_hooks(event, payload, &working_dir, &session_id)
                    .await
            } else {
                crate::hooks::AggregatedHookResult::new()
            }
        }
    }

    /// Set the checkpoint store for file snapshots before Write/Edit operations.
    pub fn set_checkpoint_store(&mut self, store: CheckpointStore) {
        self.checkpoint_store = Some(Arc::new(Mutex::new(store)));
    }

    /// Set the plan store for plan persistence.
    pub fn set_plan_store(&mut self, store: PlanStore) {
        self.plan_store = Some(store);
    }

    /// Set the memory graph for per-turn injection (GAP 7) and extraction (GAP 5).
    pub fn set_memory(&mut self, memory: Arc<dyn MemoryTrait>) {
        self.memory = Some(memory);
    }

    /// Set the auto-extraction system (v0.1.23: LLM-driven fact extraction every N turns).
    pub fn set_auto_extractor(&mut self, extractor: Arc<AutoExtractor>) {
        self.auto_extractor = Some(extractor);
    }

    pub fn pending_auto_extraction_count(&self) -> usize {
        self.auto_extraction_tasks.len()
    }

    pub fn prune_finished_auto_extractions(&mut self) {
        let mut pending = Vec::with_capacity(self.auto_extraction_tasks.len());
        for handle in self.auto_extraction_tasks.drain(..) {
            if handle.is_finished() {
                drop(handle);
            } else {
                pending.push(handle);
            }
        }
        self.auto_extraction_tasks = pending;
    }

    pub async fn flush_auto_extractions(&mut self, timeout: std::time::Duration) -> usize {
        let mut handles = std::mem::take(&mut self.auto_extraction_tasks);
        let pending = handles.len();
        if pending == 0 {
            return 0;
        }
        let deadline = std::time::Instant::now() + timeout;
        for mut handle in handles.drain(..) {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                tracing::warn!("auto-extraction task did not finish before shutdown timeout");
                handle.abort();
                let _ = handle.await;
                continue;
            }

            let timeout = tokio::time::sleep(remaining);
            tokio::pin!(timeout);
            tokio::select! {
                result = &mut handle => {
                    let _ = result;
                }
                _ = &mut timeout => {
                    tracing::warn!("auto-extraction task did not finish before shutdown timeout");
                    handle.abort();
                    let _ = handle.await;
                }
            }
        }
        pending
    }

    /// Wire the GNN auto-trainer's `record_memory(n)` hook.
    ///
    /// archon-core cannot depend on archon-pipeline directly (cycle), so the
    /// binary builds an AutoTrainer via `auto_trainer_runtime::build_and_spawn_auto_trainer`
    /// and injects this closure pointing at it. Called from the auto-extraction
    /// memory store path (line ~2659) and inner-voice snapshot path (line ~2308).
    pub fn set_record_memory_callback(&mut self, cb: Arc<dyn Fn(u64) + Send + Sync>) {
        self.record_memory_callback = Some(cb);
    }

    /// Wire the GNN auto-trainer's `record_correction()` hook.
    /// Called from `detect_and_record_correction` after a successful record.
    pub fn set_record_correction_callback(&mut self, cb: Arc<dyn Fn() + Send + Sync>) {
        self.record_correction_callback = Some(cb);
    }

    /// Wire governed-learning UserCorrected event emission.
    ///
    /// Called from the binary/pipeline layer so archon-core does not import
    /// archon-learning directly.
    pub fn set_record_user_correction_event_callback(
        &mut self,
        cb: Arc<dyn Fn(UserCorrectionEventPayload) + Send + Sync>,
    ) {
        self.record_user_correction_event_callback = Some(cb);
    }

    /// Wire reasoning-quality visible-turn emission.
    pub fn set_record_reasoning_turn_callback(
        &mut self,
        cb: Arc<dyn Fn(ReasoningTurnEventPayload) + Send + Sync>,
    ) {
        self.record_reasoning_turn_callback = Some(cb);
    }

    /// Wire the personality-mirror update hook (TASK #245).
    ///
    /// Called from every InnerVoice write site (per-tool, per-turn, user
    /// correction) immediately after mutation, while still holding the
    /// async-Mutex guard. The binary captures a sync-Mutex mirror of
    /// InnerVoice so a panic hook (which has no tokio runtime) can read
    /// the latest state to build a snapshot.
    pub fn set_inner_voice_change_callback(&mut self, cb: Arc<dyn Fn(&InnerVoice) + Send + Sync>) {
        self.inner_voice_change_callback = Some(cb);
    }

    /// Current turn number (for AutoCapture indexing).
    pub fn turn_number(&self) -> u64 {
        self.turn_number
    }

    /// Access the memory handle, if set (for AutoCapture storage).
    pub fn memory_handle(&self) -> Option<&Arc<dyn MemoryTrait>> {
        self.memory.as_ref()
    }

    /// Restore conversation state from previously saved messages.
    /// Used for session resume (`--resume <id>`).
    pub fn restore_conversation(&mut self, messages: Vec<serde_json::Value>) {
        self.state.messages = messages;
    }

    /// Set the auto-mode evaluator for permission classification (GAP 6).
    pub fn set_auto_evaluator(&mut self, evaluator: AutoModeEvaluator) {
        self.auto_evaluator = Some(evaluator);
    }
}
