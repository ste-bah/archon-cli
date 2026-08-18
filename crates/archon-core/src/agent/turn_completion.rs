use super::*;

impl Agent {
    pub(super) async fn complete_turn_without_tools(
        &mut self,
        user_input: &str,
        turn_input_tokens: u64,
        turn_output_tokens: u64,
        turn_cache_creation: u64,
        turn_cache_read: u64,
        active_model: &str,
    ) {
        // No tool calls -- turn is complete
        // Update shared session stats for /status and /cost
        {
            let mut stats = self.session_stats.lock().await;
            stats.input_tokens = self.state.total_input_tokens;
            stats.output_tokens = self.state.total_output_tokens;
            stats.turn_count = self.turn_number;
            // Update cache statistics from this turn
            stats
                .cache_stats
                .update(turn_cache_creation, turn_cache_read, turn_input_tokens);
            stats.session_cost = crate::cost::estimate_session_cost_usd(
                active_model,
                stats.input_tokens,
                stats.output_tokens,
                stats.cache_stats.cache_creation_tokens,
                stats.cache_stats.cache_read_tokens,
            );
        }

        // Apply turn completion to inner voice (energy decay, turn counter).
        if let Some(iv) = &self.inner_voice {
            let mut iv_guard = iv.lock().await;
            iv_guard.on_turn_complete();
            // TASK #245: keep panic-mirror in lock-step.
            if let Some(ref cb) = self.inner_voice_change_callback {
                cb(&iv_guard);
            }
        }

        self.send_event(AgentEvent::TurnComplete {
            input_tokens: turn_input_tokens,
            output_tokens: turn_output_tokens,
            cache_creation_tokens: turn_cache_creation,
            cache_read_tokens: turn_cache_read,
        })
        .await;

        // Try arithmetic before any segment is closed for summarisation (#189
        // Phase 8). A file read three times unchanged, an error retried
        // successfully, a result already spilled to disk — all removable
        // without judgement, and a summary of them costs a request and a wait
        // to reach the same place. Runs here, at turn end, because this is the
        // production compaction hook and the only one holding `&mut self`.
        // Only under pressure: rewriting history every turn would change what
        // the model sees whether or not anything needed reclaiming.
        if self.staged_compaction_due(active_model) {
            let telemetry = self.compaction_telemetry_for(active_model);
            // The return value says whether the threshold was cleared, which
            // the method logs itself. Nothing here branches on it: if pruning
            // was enough, `staged_compaction_due` simply stops being true and
            // no summary is ever assembled.
            let _ = self.prune_context_mechanically(&telemetry, false);
        }

        self.close_completed_compaction_segment(active_model);

        // CRIT-14 (ITEM 4): Decay rule scores every 50 turns.
        if self.turn_number.is_multiple_of(50)
            && let Some(ref graph) = self.memory
        {
            let engine = RulesEngine::new(graph.as_ref());
            if let Err(e) = engine.decay_scores(1.0) {
                tracing::warn!("rules decay_scores failed: {e}");
            }
        }

        // Detect user corrections and record them in the memory graph.
        // The handle is cloned rather than borrowed because recording now also
        // buffers what it captured, so the call needs `&mut self`.
        //
        // The buffer length is sampled either side of the call because it is
        // cumulative until the next extraction: the delta is what this turn
        // contributed, and the reflection trigger below needs "was corrected
        // now", not "has been corrected since some earlier turn".
        let corrections_before = self.corrections_since_extraction.len();
        // The classifier's verdict travels with the flag. `user_corrected` says
        // the live path recorded a correction; the verdict says how sure the
        // detector was, which is what the reflection trigger weighs.
        let classification = match self.memory.clone() {
            Some(graph) => Some(self.detect_and_record_correction(user_input, &graph).await),
            None => None,
        };
        let user_corrected = self.corrections_since_extraction.len() > corrections_before;

        // Issues #76 and #81: join this turn to the shadow plan recorded at
        // turn start, and reflect if the comparison, the tool failures, or the
        // correction tripped a trigger. Bounded and failure-tolerant: the turn
        // is already complete and nothing here can change it.
        self.complete_cognitive_shadow_turn(user_input, user_corrected, classification.as_ref())
            .await;

        // R2 slice, item 7: a turn that ran tools is a repeated opportunity for
        // every correction this session already attributed. Recorded after the
        // correction pass so a correction detected on THIS turn cannot count its
        // own turn as a chance to have recurred.
        self.record_attribution_followup(user_input).await;

        // GAP 5: Auto-memory extraction check
        self.extraction_state.record_turn();
        if should_extract(
            &self.extraction_config,
            &self.extraction_state,
            self.turn_number as usize,
        ) {
            self.trigger_memory_extraction();
        }
    }
}
