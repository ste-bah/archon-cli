use super::*;

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
    pub(super) fn inject_memories(&mut self) -> Vec<serde_json::Value> {
        let mut system = self.config.system_prompt.clone();

        let graph = match self.memory {
            Some(ref g) => g,
            None => return system,
        };

        // Collect recent user messages as context for recall
        let context: Vec<String> = self
            .state
            .messages
            .iter()
            .rev()
            .filter(|m| m["role"].as_str() == Some("user"))
            .take(3)
            .filter_map(|m| m["content"].as_str().map(|s| s.to_string()))
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

        match self.memory_injector.inject(graph.as_ref(), &context, 500) {
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
        let ctx_joined = context.join(" ");
        let tracker = CorrectionTracker::new(graph.as_ref());
        match tracker.recall_corrections(&ctx_joined, 5) {
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

    /// Detect correction patterns in user input and record via CorrectionTracker.
    pub(super) async fn detect_and_record_correction(
        &self,
        user_input: &str,
        graph: &Arc<dyn MemoryTrait>,
    ) {
        let lower = user_input.to_lowercase();
        let correction_type = if lower.starts_with("no,")
            || lower.starts_with("no ")
            || lower.starts_with("wrong")
            || lower.starts_with("that's wrong")
            || lower.starts_with("that is wrong")
        {
            CorrectionType::FactualError
        } else if lower.contains("i said")
            || lower.contains("i already told you")
            || lower.contains("i already asked")
            || lower.contains("as i mentioned")
        {
            CorrectionType::RepeatedInstruction
        } else if lower.starts_with("don't ")
            || lower.starts_with("do not ")
            || lower.starts_with("stop ")
            || lower.contains("never do that")
        {
            CorrectionType::DidForbiddenAction
        } else if lower.contains("didn't ask")
            || lower.contains("did not ask")
            || lower.contains("without permission")
            || lower.contains("without asking")
        {
            CorrectionType::ActedWithoutPermission
        } else if lower.contains("instead,")
            || lower.contains("should have")
            || lower.contains("better approach")
            || lower.contains("use this instead")
        {
            CorrectionType::ApproachCorrection
        } else {
            return; // No correction pattern detected.
        };

        let tracker = CorrectionTracker::new(graph.as_ref());
        let context = format!("turn:{}", self.turn_number);
        let engine = RulesEngine::new(graph.as_ref());
        let rules = match engine.get_rules_sorted() {
            Ok(rules) => rules,
            Err(error) => {
                tracing::warn!("rule lookup failed during correction handling: {error}");
                return;
            }
        };
        let linked_rule_id = select_relevant_rule(user_input, &rules).map(|rule| rule.id.clone());
        let correction = match tracker.record_correction(
            correction_type,
            user_input,
            &context,
            linked_rule_id.as_deref(),
        ) {
            Ok(correction) => {
                // Reference: archon-pipeline/src/learning/gnn/auto_trainer.rs::record_correction.
                // Closure-injection avoids cycle (archon-core cannot import archon-pipeline).
                if let Some(ref cb) = self.record_correction_callback {
                    cb();
                }
                Some(correction)
            }
            Err(e) => {
                tracing::warn!("failed to record correction: {e}");
                None
            }
        };

        // CRIT-15 (ITEM 5): Notify inner voice of user correction.
        if let Some(ref iv) = self.inner_voice
            && let Ok(mut iv) = iv.try_lock()
        {
            iv.on_user_correction();
            // TASK #245: keep panic-mirror in lock-step (inside the same
            // try_lock guard, so mirror cannot drift relative to actual).
            if let Some(ref cb) = self.inner_voice_change_callback {
                cb(&iv);
            }
        }

        if let Some(ref cb) = self.record_user_correction_event_callback {
            let payload = UserCorrectionEventPayload {
                correction_type: format!("{correction_type:?}"),
                top_rule_id: correction.and_then(|record| record.rule_id),
                user_input_excerpt: user_correction_excerpt(user_input),
                session_context: context,
            };
            self.fire_before_learning_event_hook("UserCorrected", &payload)
                .await;
            cb(payload.clone());
            self.fire_after_learning_event_hook("UserCorrected", &payload)
                .await;
        }
    }

    /// GAP 5: Trigger memory extraction in the background.
    pub(super) fn trigger_memory_extraction(&mut self) {
        let graph = match self.memory {
            Some(ref g) => Arc::clone(g),
            None => return,
        };

        // Collect last N messages for extraction
        let messages: Vec<String> = self
            .state
            .messages
            .iter()
            .rev()
            .take(10)
            .filter_map(|m| {
                let role = m["role"].as_str().unwrap_or("unknown");
                let content = m["content"].as_str().unwrap_or("");
                if content.is_empty() {
                    return None;
                }
                Some(format!("{role}: {content}"))
            })
            .collect();

        if messages.is_empty() {
            return;
        }

        let session_id = self.config.session_id.clone();
        let turn = self.turn_number as usize;
        let attribution = self.config.runtime_attribution_extra(
            "memory_extraction",
            "memory_extraction",
            Some(self.turn_number),
            None,
            None,
        );
        let client = Arc::clone(&self.client);
        let model = self.config.model.clone();
        // Reference: auto_trainer_runtime.rs — closure pointing at AutoTrainer.record_memories.
        let mem_cb = self.record_memory_callback.as_ref().map(Arc::clone);

        // Record extraction so we don't fire again immediately
        self.extraction_state.record_extraction(turn);

        // Run extraction in background via a real LLM call
        tokio::spawn(async move {
            let prompt = build_extraction_prompt(&messages);

            let request = LlmRequest {
                model,
                max_tokens: 1024,
                system: vec![serde_json::json!({
                    "type": "text",
                    "text": "You extract structured memories from conversations. Return ONLY a JSON array."
                })],
                messages: vec![serde_json::json!({
                    "role": "user",
                    "content": prompt,
                })],
                tools: Vec::new(),
                thinking: None,
                speed: None,
                effort: None,
                extra: attribution,
                request_origin: Some("memory_extraction".into()),
                reasoning_encrypted: None,
            };

            match client.stream(request).await {
                Ok(mut rx) => {
                    let mut response_text = String::new();
                    while let Some(event) = rx.recv().await {
                        if let StreamEvent::TextDelta { text, .. } = event {
                            response_text.push_str(&text);
                        }
                    }

                    let extracted = parse_extraction_response(&response_text).unwrap_or_default();
                    if !extracted.is_empty() {
                        match store_extracted(graph.as_ref(), &extracted, &session_id) {
                            Ok(count) => {
                                tracing::info!("auto-extracted {count} memories at turn {turn}");
                                // Reference: auto_trainer.rs::record_memories — bumps the
                                // GNN auto-trainer's memory counter so triggers fire when
                                // the configured threshold is met.
                                if let Some(ref cb) = mem_cb {
                                    cb(count as u64);
                                }
                            }
                            Err(e) => tracing::warn!("memory extraction storage failed: {e}"),
                        }
                    } else {
                        tracing::debug!("no memories extracted at turn {turn}");
                    }
                }
                Err(e) => {
                    tracing::warn!("memory extraction API call failed: {e}");
                }
            }
        });
    }
}

const MATCH_THRESHOLD: f64 = 0.25;
const MIN_MEANINGFUL_OVERLAP: usize = 2;
const COMMON_CORRECTION_TOKENS: &[&str] = &[
    "a", "an", "and", "before", "do", "for", "i", "instead", "is", "it", "no", "not", "of", "or",
    "should", "that", "the", "this", "to", "use", "was", "with", "you",
];

fn select_relevant_rule<'a>(
    correction: &str,
    rules: &'a [archon_consciousness::rules::BehavioralRule],
) -> Option<&'a archon_consciousness::rules::BehavioralRule> {
    let correction_token_set = correction_tokens(correction);
    rules
        .iter()
        .filter_map(|rule| {
            let rule_tokens = correction_tokens(&rule.text);
            let overlap = correction_token_set.intersection(&rule_tokens).count();
            let union = correction_token_set.union(&rule_tokens).count();
            let jaccard = overlap as f64 / union.max(1) as f64;
            let dice = (2 * overlap) as f64
                / (correction_token_set.len() + rule_tokens.len()).max(1) as f64;
            let relevance = (jaccard + dice) / 2.0;
            (overlap >= MIN_MEANINGFUL_OVERLAP
                && jaccard >= MATCH_THRESHOLD
                && dice >= MATCH_THRESHOLD)
                .then_some((rule, relevance))
        })
        .max_by(
            |(left_rule, left_similarity), (right_rule, right_similarity)| {
                left_similarity
                    .total_cmp(right_similarity)
                    .then_with(|| left_rule.score.total_cmp(&right_rule.score))
                    .then_with(|| right_rule.id.cmp(&left_rule.id))
            },
        )
        .map(|(rule, _)| rule)
}

fn correction_tokens(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.len() > 1 && !COMMON_CORRECTION_TOKENS.contains(&token.as_str()))
        .collect()
}
