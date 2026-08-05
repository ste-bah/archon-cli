use super::*;

/// Token budget handed to [`MemoryInjector::inject`] each turn.
const MEMORY_INJECTION_BUDGET_TOKENS: usize = 500;

/// How many past corrections are surfaced into the system prompt.
const RECALLED_CORRECTION_LIMIT: usize = 5;

/// Characters of a user message used to build the memory recall query.
///
/// Recall wants a topical signal, not a transcript. 600 characters is roughly
/// 90 words -- comfortably enough to characterise a request, and far below the
/// point where an FTS `OR` over the distinct terms stops being cheap. It also
/// matches the excerpt length already used for reasoning evidence below, so a
/// reader sees one bound rather than two arbitrary ones.
const RECALL_QUERY_EXCERPT_CHARS: usize = 600;

/// Take the leading, character-safe excerpt of `text` for the recall query.
///
/// Splits on a `char` boundary rather than a byte index, so a multi-byte
/// character straddling the cut cannot panic.
fn recall_query_excerpt(text: &str) -> String {
    text.chars().take(RECALL_QUERY_EXCERPT_CHARS).collect()
}

/// What the blocking memory queries produced, carried back to the async side.
struct RecalledMemories {
    injected: Result<String, archon_memory::MemoryError>,
    corrections: Result<
        Vec<archon_consciousness::corrections::Correction>,
        archon_consciousness::corrections::CorrectionError,
    >,
}

impl Agent {
    /// Append the inner voice `<inner_voice>` block to the system prompt
    /// for this turn, if the feature is enabled.
    pub(super) async fn inject_inner_voice(&self, system: &mut Vec<serde_json::Value>) {
        let iv = match &self.inner_voice {
            Some(iv) => iv,
            None => return,
        };
        let block = iv.lock().await.to_prompt_block();
        system.push(serde_json::json!({
            "type": "text",
            "text": block,
        }));
    }

    /// Inject critical system reminder into the system prompt (AGT-022).
    /// Re-injected every turn, wrapped in `<system-reminder>` tags.
    pub(super) fn inject_critical_reminder(&self, system: &mut Vec<serde_json::Value>) {
        if let Some(ref reminder) = self.critical_system_reminder {
            system.push(serde_json::json!({
                "type": "text",
                "text": format!("<system-reminder>{reminder}</system-reminder>"),
            }));
        }
    }

    pub(super) fn inject_turn_requirements(&self, system: &mut Vec<serde_json::Value>) {
        if let Some(ref reminder) = self.cognitive_executive_reminder {
            system.push(serde_json::json!({
                "type": "text",
                "text": format!("<cognitive-executive>{reminder}</cognitive-executive>"),
            }));
        }
        if let Some(ref reminder) = self.turn_requirement_reminder {
            system.push(serde_json::json!({
                "type": "text",
                "text": format!("<guardrail-requirements>{reminder}</guardrail-requirements>"),
            }));
        }
    }

    /// GAP 7: Inject recalled memories into the system prompt for this turn.
    ///
    /// The memory work is synchronous, can be slow on a large store, and
    /// contains no `.await` points. Called inline from an async fn it pinned a
    /// tokio worker thread for the whole scan and the task never yielded, so
    /// the cancellation token set by the caller was never polled and Ctrl+C
    /// could not interrupt the stall. It now runs on the blocking pool: the
    /// executor stays free, and awaiting the join handle is the yield point at
    /// which cancellation finally becomes observable.
    pub(super) async fn inject_memories(&mut self) -> Vec<serde_json::Value> {
        let mut system = self.config.system_prompt.clone();

        let graph = match self.memory {
            Some(ref g) => Arc::clone(g),
            None => return system,
        };

        // Collect recent user messages as context for recall, bounded.
        //
        // A slash command's user message is its whole injected skill template:
        // 21 KB, ~3,200 words, ~1,500 distinct terms once deduplicated. Every
        // one of those became a branch of an FTS `OR`, and Cozo evaluates every
        // branch in full. Measured on a 1.7 GB store: 12.4 seconds to return
        // ZERO corrections -- pure cost, and the zero is the point. Boilerplate
        // is not what anyone wants memories recalled against, so a query built
        // from it is both ruinous and meaningless.
        //
        // Truncating per message rather than dropping long ones keeps a genuine
        // long prompt working: the head of a request carries its intent, and
        // recall only ever needed a topical signal, not the full text.
        let context: Vec<String> = self
            .state
            .messages
            .iter()
            .rev()
            .filter(|m| m["role"].as_str() == Some("user"))
            .take(3)
            .filter_map(|m| m["content"].as_str().map(recall_query_excerpt))
            .filter(|excerpt| !excerpt.trim().is_empty())
            .collect();

        if context.is_empty() {
            return system;
        }
        self.reasoning_evidence_refs
            .push(ReasoningEvidenceEventPayload {
                evidence_id: format!("chat_history:turn:{}", self.turn_number),
                kind: "chat_history".to_string(),
                entity_key: Some("recent_user_context".to_string()),
                output_hash: None,
                redacted_excerpt: Some(context.join("\n").chars().take(600).collect()),
                created_at: chrono::Utc::now().to_rfc3339(),
            });

        let recalled = match self.recall_off_executor(graph, context).await {
            Some(recalled) => {
                // Bank a successful recall for the rest of this turn. Only Ok
                // is cached: an error is a transient failure worth retrying on
                // the next round, not an answer.
                if let Ok(corrections) = &recalled.corrections {
                    self.recalled_corrections = Some((self.turn_number, corrections.clone()));
                }
                recalled
            }
            // Cancelled, or the blocking task died. Either way there is nothing
            // to inject and the caller is about to unwind anyway.
            None => return system,
        };

        match recalled.injected {
            Ok(memories_text) if !memories_text.is_empty() => {
                let surfaced = memories_text
                    .lines()
                    .filter(|line| line.trim_start().starts_with("- "))
                    .count();
                self.emit_activity(
                    AgentActivityKind::MemorySurfaced,
                    AgentActivityStatus::Completed,
                    format!("surfaced {surfaced} task-relevant memories from early user context"),
                );
                system.push(serde_json::json!({
                    "type": "text",
                    "text": memories_text,
                }));
            }
            Ok(_) => {} // empty — no relevant memories
            Err(e) => {
                tracing::warn!("memory injection failed: {e}");
            }
        }

        // Inject recalled corrections relevant to the current context.
        match recalled.corrections {
            Ok(corrections) if !corrections.is_empty() => {
                let mut block = String::from(
                    "<past_corrections>\nPrevious user corrections relevant to this context:\n",
                );
                for c in &corrections {
                    block.push_str(&format!(
                        "- [{}] {}\n",
                        c.correction_type.severity_multiplier(),
                        c.content
                    ));
                }
                block.push_str("</past_corrections>");
                system.push(serde_json::json!({
                    "type": "text",
                    "text": block,
                }));
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("correction recall failed: {e}");
            }
        }

        // CLI-416/417: Inject personality and memory briefings on first turn only.
        if let Some(briefing) = self.personality_briefing.take() {
            system.push(serde_json::json!({
                "type": "text",
                "text": briefing,
            }));
        }
        if let Some(briefing) = self.memory_briefing.take() {
            self.emit_activity(
                AgentActivityKind::MemorySurfaced,
                AgentActivityStatus::Completed,
                "injected first-turn memory garden briefing",
            );
            self.reasoning_evidence_refs
                .push(ReasoningEvidenceEventPayload {
                    evidence_id: "memory_briefing:first_turn".to_string(),
                    kind: "memory".to_string(),
                    entity_key: Some("memory_briefing".to_string()),
                    output_hash: None,
                    redacted_excerpt: Some(briefing.chars().take(600).collect()),
                    created_at: chrono::Utc::now().to_rfc3339(),
                });
            system.push(serde_json::json!({
                "type": "text",
                "text": briefing,
            }));
        }

        system
    }

    /// Drop the memory injector's cached block.
    ///
    /// Uses `try_lock` rather than `lock` because the only thing that can hold
    /// this mutex is an abandoned injection still running on the blocking pool,
    /// and blocking the async executor waiting for it is precisely the failure
    /// being fixed. Losing the race is not a reason to skip the invalidation:
    /// the in-flight task is about to publish a cache entry for a context this
    /// call is invalidating, so the whole injector is replaced instead. The old
    /// one stays alive until that task finishes writing into it and is then
    /// dropped, cache and all.
    pub(super) fn invalidate_memory_injector_cache(&mut self) {
        let invalidated_in_place = match self.memory_injector.try_lock() {
            Ok(mut injector) => {
                injector.invalidate_cache();
                true
            }
            Err(_) => false,
        };
        if !invalidated_in_place {
            self.memory_injector = Arc::new(std::sync::Mutex::new(MemoryInjector::new()));
        }
    }

    /// Run the two blocking memory queries off the async executor.
    ///
    /// Returns `None` when the caller's cancellation token fired first, or when
    /// the blocking task itself failed.
    ///
    /// The injector is reached through a shared handle rather than moved in.
    /// `spawn_blocking` work cannot be cancelled, so anything moved into the
    /// closure is lost for good the moment the caller stops awaiting it —
    /// including, if it were moved, the injector and its cache.
    async fn recall_off_executor(
        &mut self,
        graph: Arc<dyn MemoryTrait>,
        context: Vec<String>,
    ) -> Option<RecalledMemories> {
        let cancel = self.config.cancel_token.clone();
        if cancel.as_ref().is_some_and(|token| token.is_cancelled()) {
            tracing::debug!("memory injection skipped: turn already cancelled");
            return None;
        }

        // Corrections are recalled ONCE per turn, not once per agent-loop
        // iteration. Measured at 8.8s per call returning zero rows, so a
        // twenty-round turn re-paid ~176 seconds answering the same question.
        // Safe because corrections are only recorded at turn END, so none can
        // appear mid-turn for this to miss -- see `recalled_corrections`.
        let turn = self.turn_number;
        let cached_corrections = match &self.recalled_corrections {
            Some((cached_turn, corrections)) if *cached_turn == turn => Some(corrections.clone()),
            _ => None,
        };

        let injector = Arc::clone(&self.memory_injector);
        let handle = tokio::task::spawn_blocking(move || {
            // Timed and reported on EVERY outcome, including the empty one.
            //
            // This runs on every turn before the provider is contacted and
            // scans a store that grows without bound. When it previously logged
            // only on error, a turn that spent 38 minutes here was
            // indistinguishable from a turn waiting on the model -- no
            // duration, no counts, and the success path silent. The elapsed
            // time IS the diagnostic, so it is emitted whether or not anything
            // surfaced; an empty result that took minutes is the case that most
            // looks like a hang.
            let started = std::time::Instant::now();
            let injected = match injector.lock() {
                Ok(mut injector) => {
                    injector.inject(graph.as_ref(), &context, MEMORY_INJECTION_BUDGET_TOKENS)
                }
                // A poisoned injector means a previous injection panicked. The
                // cache is only a memoisation, so recover rather than escalate.
                Err(poisoned) => poisoned.into_inner().inject(
                    graph.as_ref(),
                    &context,
                    MEMORY_INJECTION_BUDGET_TOKENS,
                ),
            };
            let injection_ms = started.elapsed().as_millis();
            let recall_started = std::time::Instant::now();
            let (corrections, recall_cached) = match cached_corrections {
                Some(cached) => (Ok(cached), true),
                None => {
                    let tracker = CorrectionTracker::new(graph.as_ref());
                    let recalled =
                        tracker.recall_corrections(&context.join(" "), RECALLED_CORRECTION_LIMIT);
                    (recalled, false)
                }
            };
            tracing::info!(
                injection_ms,
                injection_bytes = injected.as_ref().map(String::len).unwrap_or(0),
                recall_ms = recall_started.elapsed().as_millis(),
                recall_cached,
                recalled = corrections.as_ref().map(Vec::len).unwrap_or(0),
                "memory recall complete"
            );
            RecalledMemories {
                injected,
                corrections,
            }
        });

        let joined = match cancel {
            Some(token) => tokio::select! {
                biased;
                () = token.cancelled() => {
                    // The blocking thread cannot be pre-empted: the scan keeps
                    // running and keeps a blocking-pool thread busy until it
                    // finishes. What this buys is that the *turn* stops waiting
                    // for it, and that the executor was never blocked at all.
                    tracing::debug!("memory injection abandoned: turn cancelled");
                    return None;
                }
                joined = handle => joined,
            },
            None => handle.await,
        };

        match joined {
            Ok(recalled) => Some(recalled),
            Err(error) => {
                tracing::warn!("memory injection task failed: {error}");
                None
            }
        }
    }
}
