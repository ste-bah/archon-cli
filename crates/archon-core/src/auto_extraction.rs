use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use archon_llm::provider::{LlmProvider, LlmRequest, LlmResponse};
use archon_memory::MemoryTrait;

/// Extracts facts from recent conversation turns using an LLM and stores
/// them in the memory graph. Runs every N turns via `maybe_extract`.
pub struct AutoExtractor {
    llm: Arc<dyn LlmProvider>,
    memory: Arc<dyn MemoryTrait>,
    every_n_turns: u32,
    last_run_turn: AtomicU32,
    enabled: bool,
}

impl AutoExtractor {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        memory: Arc<dyn MemoryTrait>,
        every_n_turns: u32,
        enabled: bool,
    ) -> Self {
        Self {
            llm,
            memory,
            every_n_turns: if every_n_turns == 0 { 5 } else { every_n_turns },
            last_run_turn: AtomicU32::new(0),
            enabled,
        }
    }

    /// Invoke every turn. Runs extraction only when `current_turn - last >= every_n_turns`.
    /// Returns stored fact IDs on success, empty vec when throttled or disabled.
    pub async fn maybe_extract(
        &self,
        recent_turns: &[String],
        current_turn: u32,
        model: &str,
    ) -> Vec<String> {
        if !self.enabled {
            return vec![];
        }

        let last = self.last_run_turn.load(Ordering::Relaxed);
        if current_turn.saturating_sub(last) < self.every_n_turns {
            return vec![];
        }

        self.last_run_turn.store(current_turn, Ordering::Relaxed);

        let facts = match self.extract_facts(recent_turns, model).await {
            Ok(f) => f,
            Err(_) => return vec![],
        };

        let mut stored_ids = Vec::new();
        for fact in facts {
            let fact = fact.trim().to_string();
            if fact.is_empty() || fact.len() < 5 {
                continue;
            }
            // Dedup: skip if a similar memory already exists.
            if self.is_duplicate(&fact).await {
                continue;
            }
            match self.memory.store_memory(
                &fact,
                &fact.chars().take(80).collect::<String>(),
                archon_memory::types::MemoryType::Fact,
                0.5,
                &["auto-extracted".to_string()],
                "auto_extraction",
                "",
            ) {
                Ok(id) => stored_ids.push(id),
                Err(_) => {}
            }
        }

        stored_ids
    }

    /// Build a small extraction prompt and call the LLM.
    async fn extract_facts(
        &self,
        recent_turns: &[String],
        model: &str,
    ) -> Result<Vec<String>, String> {
        let conversation = recent_turns.join("\n\n");

        let prompt = format!(
            "Extract factual statements about the user's preferences, decisions, \
             or project state from this conversation. Return one fact per line. \
             Only extract clear, explicit facts. Do not infer or guess.\n\n\
             Conversation:\n{}\n\nFacts:",
            &conversation[..conversation.len().min(4000)]
        );

        let request = LlmRequest {
            model: model.to_string(),
            max_tokens: 500,
            system: vec![],
            messages: vec![serde_json::json!({
                "role": "user",
                "content": prompt,
            })],
            tools: vec![],
            thinking: None,
            speed: None,
            effort: None,
            extra: serde_json::Value::Null,
            request_origin: Some("auto_extraction".into()),
        };

        let response: LlmResponse = self
            .llm
            .complete(request)
            .await
            .map_err(|e| format!("LLM error: {e}"))?;

        let text: String = response
            .content
            .iter()
            .filter_map(|block| {
                block
                    .get("text")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .collect::<Vec<_>>()
            .join("");

        Ok(text
            .lines()
            .map(|l| {
                l.trim()
                    .trim_start_matches('-')
                    .trim_start_matches('*')
                    .trim()
                    .to_string()
            })
            .filter(|l| !l.is_empty())
            .collect())
    }

    /// Check whether a fact is a near-duplicate of an existing memory.
    async fn is_duplicate(&self, fact: &str) -> bool {
        match self.memory.recall_memories(fact, 3) {
            Ok(existing) => {
                let fact_lower = fact.to_lowercase();
                existing.iter().any(|m| {
                    let existing_lower = m.content.to_lowercase();
                    // Simple Jaccard-like check.
                    let fact_words: std::collections::HashSet<&str> =
                        fact_lower.split_whitespace().collect();
                    let existing_words: std::collections::HashSet<&str> =
                        existing_lower.split_whitespace().collect();
                    if fact_words.is_empty() || existing_words.is_empty() {
                        return false;
                    }
                    let intersection = fact_words.intersection(&existing_words).count();
                    let union = fact_words.union(&existing_words).count();
                    if union == 0 {
                        return false;
                    }
                    (intersection as f64 / union as f64) > 0.8
                })
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeMemory;
    impl MemoryTrait for FakeMemory {
        fn store_memory(
            &self,
            _content: &str,
            _title: &str,
            _memory_type: archon_memory::types::MemoryType,
            _importance: f64,
            _tags: &[String],
            _source_type: &str,
            _project_path: &str,
        ) -> Result<String, archon_memory::types::MemoryError> {
            Ok("test-id".into())
        }
        fn get_memory(
            &self,
            _id: &str,
        ) -> Result<archon_memory::types::Memory, archon_memory::types::MemoryError> {
            Err(archon_memory::types::MemoryError::NotFound("test".into()))
        }
        fn update_memory(
            &self,
            _id: &str,
            _content: Option<&str>,
            _tags: Option<&[String]>,
        ) -> Result<(), archon_memory::types::MemoryError> {
            Ok(())
        }
        fn update_importance(
            &self,
            _id: &str,
            _importance: f64,
        ) -> Result<(), archon_memory::types::MemoryError> {
            Ok(())
        }
        fn delete_memory(&self, _id: &str) -> Result<(), archon_memory::types::MemoryError> {
            Ok(())
        }
        fn create_relationship(
            &self,
            _from_id: &str,
            _to_id: &str,
            _rel_type: archon_memory::types::RelType,
            _context: Option<&str>,
            _strength: f64,
        ) -> Result<(), archon_memory::types::MemoryError> {
            Ok(())
        }
        fn recall_memories(
            &self,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<archon_memory::types::Memory>, archon_memory::types::MemoryError> {
            Ok(vec![])
        }
        fn search_memories(
            &self,
            _filter: &archon_memory::types::SearchFilter,
        ) -> Result<Vec<archon_memory::types::Memory>, archon_memory::types::MemoryError> {
            Ok(vec![])
        }
        fn list_recent(
            &self,
            _limit: usize,
        ) -> Result<Vec<archon_memory::types::Memory>, archon_memory::types::MemoryError> {
            Ok(vec![])
        }
        fn memory_count(&self) -> Result<usize, archon_memory::types::MemoryError> {
            Ok(0)
        }
        fn clear_all(&self) -> Result<usize, archon_memory::types::MemoryError> {
            Ok(0)
        }
        fn get_related_memories(
            &self,
            _id: &str,
            _depth: u32,
        ) -> Result<Vec<archon_memory::types::Memory>, archon_memory::types::MemoryError> {
            Ok(vec![])
        }
    }

    #[test]
    fn test_disabled_returns_empty() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Test that constructor stores enabled flag correctly.
        let extractor = AutoExtractor::new(Arc::new(FakeLlm), Arc::new(FakeMemory), 5, false);
        assert!(!extractor.enabled);
        // Throttle check: turn 5 but disabled -> empty result
        let result = rt.block_on(extractor.maybe_extract(&[], 5, "test"));
        assert!(result.is_empty());
    }

    // Fake LLM provider for testing — always returns an error so we can test
    // error handling without real API calls.
    struct FakeLlm;
    #[async_trait::async_trait]
    impl LlmProvider for FakeLlm {
        fn name(&self) -> &str {
            "fake"
        }
        fn models(&self) -> Vec<archon_llm::provider::ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _request: LlmRequest,
        ) -> Result<
            tokio::sync::mpsc::Receiver<archon_llm::streaming::StreamEvent>,
            archon_llm::provider::LlmError,
        > {
            Err(archon_llm::provider::LlmError::Http("fake".into()))
        }
        async fn complete(
            &self,
            _request: LlmRequest,
        ) -> Result<LlmResponse, archon_llm::provider::LlmError> {
            Err(archon_llm::provider::LlmError::Http("fake".into()))
        }
        fn supports_feature(&self, _feature: archon_llm::provider::ProviderFeature) -> bool {
            false
        }
    }

    #[test]
    fn test_throttled_by_turn_count() {
        let extractor = AutoExtractor::new(Arc::new(FakeLlm), Arc::new(FakeMemory), 5, true);
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Turn 3 < every_n_turns=5: should not trigger extraction.
        let result = rt.block_on(extractor.maybe_extract(&[], 3, "test"));
        assert!(result.is_empty());
    }

    #[test]
    fn test_last_run_turn_updated() {
        let extractor = AutoExtractor::new(Arc::new(FakeLlm), Arc::new(FakeMemory), 5, true);
        let rt = tokio::runtime::Runtime::new().unwrap();
        // Turn 5 >= 5: will attempt extraction, LLM fails, but turn gets updated.
        let _ = rt.block_on(extractor.maybe_extract(&["user: hello".into()], 5, "test"));
        assert_eq!(extractor.last_run_turn.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn test_every_n_turns_defaults_to_5() {
        let extractor = AutoExtractor::new(Arc::new(FakeLlm), Arc::new(FakeMemory), 0, true);
        assert_eq!(extractor.every_n_turns, 5);
    }

    #[test]
    fn test_memory_config_enabled_flag() {
        let extractor = AutoExtractor::new(Arc::new(FakeLlm), Arc::new(FakeMemory), 5, false);
        assert!(!extractor.enabled);
    }

    #[test]
    fn test_llm_error_returns_empty() {
        let extractor = AutoExtractor::new(Arc::new(FakeLlm), Arc::new(FakeMemory), 5, true);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result =
            rt.block_on(extractor.maybe_extract(&["user: test message".into()], 5, "test"));
        assert!(result.is_empty());
    }
}
