#[tokio::test]
async fn correction_detection_links_the_relevant_rule_not_the_global_top_rule() {
    let mut agent = test_agent();
    let graph = MemoryGraph::in_memory().expect("in-memory graph");
    let (unrelated_rule_id, relevant_rule_id) = {
        let engine = RulesEngine::new(&graph);
        let unrelated = engine
            .add_rule("prefer concise corrections", RuleSource::UserDefined)
            .expect("seed unrelated rule");
        for _ in 0..10 {
            engine
                .reinforce_rule(&unrelated.id)
                .expect("reinforce unrelated");
        }
        let relevant = engine
            .add_rule("Always ask before modifying files", RuleSource::UserDefined)
            .expect("seed relevant rule");
        (unrelated.id, relevant.id)
    };
    let graph: Arc<dyn MemoryTrait> = Arc::new(graph);

    let correction_count = Arc::new(AtomicUsize::new(0));
    let correction_count_cb = Arc::clone(&correction_count);
    agent.set_record_correction_callback(Arc::new(move || {
        correction_count_cb.fetch_add(1, Ordering::SeqCst);
    }));

    let iv = Arc::new(Mutex::new(InnerVoice::new()));
    agent.set_inner_voice(Arc::clone(&iv));

    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_cb = Arc::clone(&captured);
    agent.set_record_user_correction_event_callback(Arc::new(move |payload| {
        captured_cb.lock().unwrap().push(payload);
    }));

    agent
        .detect_and_record_correction(
            "Use this instead: always ask before editing config files.",
            &graph,
        )
        .await;

    assert_eq!(correction_count.load(Ordering::SeqCst), 1);
    let iv = iv.try_lock().expect("inner voice lock");
    assert_eq!(iv.corrections_received, 1);
    assert!((iv.confidence - 0.6).abs() < f32::EPSILON);

    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].correction_type, "ApproachCorrection");
    assert_eq!(
        captured[0].top_rule_id.as_deref(),
        Some(relevant_rule_id.as_str())
    );
    assert_ne!(
        captured[0].top_rule_id.as_deref(),
        Some(unrelated_rule_id.as_str())
    );
    assert!(!captured[0].user_input_excerpt.is_empty());
    assert!(captured[0].user_input_excerpt.chars().count() <= 200);

    // The finding this test exists for is WHICH rule the correction was linked
    // to, which the payload above reads back from the graph. The scores are the
    // second statement, and since R2 they say something different: this agent
    // has no cognitive store, so the correction could not be attributed, so
    // nothing was reinforced. Under the pre-R2 code the relevant rule would sit
    // at 60.0 here on the strength of a phrase match alone.
    let relevant = graph
        .get_memory(&relevant_rule_id)
        .expect("get relevant rule");
    let unrelated = graph
        .get_memory(&unrelated_rule_id)
        .expect("get unrelated rule");
    assert!(
        (relevant.importance - 50.0).abs() < f64::EPSILON,
        "an unattributed correction must not reinforce, got {}",
        relevant.importance
    );
    assert!((unrelated.importance - 100.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn correction_matching_prefers_specific_rule_over_single_token_data_candidate() {
    let mut agent = test_agent();
    let graph = MemoryGraph::in_memory().expect("in-memory graph");
    let (generic_id, specific_id) = {
        let engine = RulesEngine::new(&graph);
        let generic = engine
            .add_rule("data", RuleSource::UserDefined)
            .expect("add generic rule");
        for _ in 0..11 {
            engine
                .reinforce_rule(&generic.id)
                .expect("reinforce generic rule");
        }
        let specific = engine
            .add_rule(
                "preserve records in data processing pipeline",
                RuleSource::UserDefined,
            )
            .expect("add specific rule");
        (generic.id, specific.id)
    };
    let graph: Arc<dyn MemoryTrait> = Arc::new(graph);
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_callback = Arc::clone(&captured);
    agent.set_record_user_correction_event_callback(Arc::new(move |payload| {
        captured_callback.lock().expect("lock events").push(payload);
    }));

    agent
        .detect_and_record_correction(
            "Use this instead: data processing pipeline corrupts records.",
            &graph,
        )
        .await;

    let events = captured.lock().expect("lock events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].top_rule_id.as_deref(), Some(specific_id.as_str()));
    assert_ne!(events[0].top_rule_id.as_deref(), Some(generic_id.as_str()));
}

#[tokio::test]
async fn correction_matching_does_not_link_an_exact_single_token_rule() {
    let mut agent = test_agent();
    let graph = MemoryGraph::in_memory().expect("in-memory graph");
    let generic_id = RulesEngine::new(&graph)
        .add_rule("data", RuleSource::UserDefined)
        .expect("add generic rule")
        .id;
    let graph: Arc<dyn MemoryTrait> = Arc::new(graph);
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_callback = Arc::clone(&captured);
    agent.set_record_user_correction_event_callback(Arc::new(move |payload| {
        captured_callback.lock().expect("lock events").push(payload);
    }));

    agent
        .detect_and_record_correction("Use this instead: data.", &graph)
        .await;

    let events = captured.lock().expect("lock events");
    assert_eq!(events.len(), 1);
    assert_ne!(events[0].top_rule_id.as_deref(), Some(generic_id.as_str()));
    assert!(
        events[0].top_rule_id.is_some(),
        "a correction without a relevant rule should create and link a derived rule"
    );
}

#[tokio::test]
async fn correction_matching_creates_derived_rule_when_existing_candidate_is_weak() {
    let mut agent = test_agent();
    let graph = MemoryGraph::in_memory().expect("in-memory graph");
    let generic_id = RulesEngine::new(&graph)
        .add_rule("data", RuleSource::UserDefined)
        .expect("add generic rule")
        .id;
    let graph: Arc<dyn MemoryTrait> = Arc::new(graph);
    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_callback = Arc::clone(&captured);
    agent.set_record_user_correction_event_callback(Arc::new(move |payload| {
        captured_callback.lock().expect("lock events").push(payload);
    }));

    agent
        .detect_and_record_correction("Use this instead: preserve permission boundaries.", &graph)
        .await;

    let events = captured.lock().expect("lock events");
    assert_eq!(events.len(), 1);
    assert_ne!(events[0].top_rule_id.as_deref(), Some(generic_id.as_str()));
    assert!(
        events[0].top_rule_id.is_some(),
        "derived rule should be linked"
    );
}

struct RuleLookupFailingMemory {
    inner: MemoryGraph,
    mutations: AtomicUsize,
}

impl RuleLookupFailingMemory {
    fn new() -> Self {
        Self {
            inner: MemoryGraph::in_memory().expect("in-memory graph"),
            mutations: AtomicUsize::new(0),
        }
    }

    fn record_mutation(&self) {
        self.mutations.fetch_add(1, Ordering::SeqCst);
    }
}

impl MemoryTrait for RuleLookupFailingMemory {
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: archon_memory::MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, archon_memory::MemoryError> {
        self.record_mutation();
        self.inner.store_memory(
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
    }

    fn store_memory_with_id_outcome(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: archon_memory::MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<archon_memory::StoreMemoryOutcome, archon_memory::MemoryError> {
        self.record_mutation();
        self.inner.store_memory_with_id_outcome(
            id,
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
    }

    fn store_memory_with_id(
        &self,
        id: &str,
        content: &str,
        title: &str,
        memory_type: archon_memory::MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<archon_memory::Memory, archon_memory::MemoryError> {
        self.record_mutation();
        self.inner.store_memory_with_id(
            id,
            content,
            title,
            memory_type,
            importance,
            tags,
            source_type,
            project_path,
        )
    }

    fn get_memory(&self, id: &str) -> Result<archon_memory::Memory, archon_memory::MemoryError> {
        self.inner.get_memory(id)
    }

    fn inspect_memory(
        &self,
        id: &str,
    ) -> Result<archon_memory::Memory, archon_memory::MemoryError> {
        self.inner.inspect_memory(id)
    }

    fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<(), archon_memory::MemoryError> {
        self.record_mutation();
        self.inner.update_memory(id, content, tags)
    }

    fn apply_importance_delta(
        &self,
        id: &str,
        delta: f64,
        provenance_id: &str,
    ) -> Result<archon_memory::Memory, archon_memory::MemoryError> {
        self.record_mutation();
        self.inner.apply_importance_delta(id, delta, provenance_id)
    }

    fn has_importance_application(
        &self,
        memory_id: &str,
        provenance_id: &str,
    ) -> Result<bool, archon_memory::MemoryError> {
        self.inner
            .has_importance_application(memory_id, provenance_id)
    }

    fn delete_memory(&self, id: &str) -> Result<(), archon_memory::MemoryError> {
        self.record_mutation();
        self.inner.delete_memory(id)
    }

    fn create_relationship(
        &self,
        from_id: &str,
        to_id: &str,
        rel_type: archon_memory::RelType,
        context: Option<&str>,
        strength: f64,
    ) -> Result<(), archon_memory::MemoryError> {
        self.record_mutation();
        self.inner
            .create_relationship(from_id, to_id, rel_type, context, strength)
    }

    fn recall_memories(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<archon_memory::Memory>, archon_memory::MemoryError> {
        self.inner.recall_memories(query, limit)
    }

    fn search_memories(
        &self,
        filter: &archon_memory::SearchFilter,
    ) -> Result<Vec<archon_memory::Memory>, archon_memory::MemoryError> {
        if filter.memory_type == Some(archon_memory::MemoryType::Rule) {
            return Err(archon_memory::MemoryError::Database(
                "rule lookup unavailable".to_string(),
            ));
        }
        self.inner.search_memories(filter)
    }

    fn list_recent(
        &self,
        limit: usize,
    ) -> Result<Vec<archon_memory::Memory>, archon_memory::MemoryError> {
        self.inner.list_recent(limit)
    }

    fn memory_count(&self) -> Result<usize, archon_memory::MemoryError> {
        self.inner.memory_count()
    }

    fn clear_all(&self) -> Result<usize, archon_memory::MemoryError> {
        self.record_mutation();
        self.inner.clear_all()
    }

    fn get_related_memories(
        &self,
        id: &str,
        depth: u32,
    ) -> Result<Vec<archon_memory::Memory>, archon_memory::MemoryError> {
        self.inner.get_related_memories(id, depth)
    }
}

#[tokio::test]
async fn correction_detection_skips_mutation_when_rule_lookup_fails() {
    let mut agent = test_agent();
    let graph = Arc::new(RuleLookupFailingMemory::new());
    let memory: Arc<dyn MemoryTrait> = graph.clone();
    let correction_callbacks = Arc::new(AtomicUsize::new(0));
    let correction_callbacks_for_callback = Arc::clone(&correction_callbacks);
    agent.set_record_correction_callback(Arc::new(move || {
        correction_callbacks_for_callback.fetch_add(1, Ordering::SeqCst);
    }));

    let inner_voice = Arc::new(Mutex::new(InnerVoice::new()));
    agent.set_inner_voice(Arc::clone(&inner_voice));

    let event_callbacks = Arc::new(AtomicUsize::new(0));
    let event_callbacks_for_callback = Arc::clone(&event_callbacks);
    agent.set_record_user_correction_event_callback(Arc::new(move |_| {
        event_callbacks_for_callback.fetch_add(1, Ordering::SeqCst);
    }));

    agent
        .detect_and_record_correction("Use this instead: preserve permission boundaries.", &memory)
        .await;

    assert_eq!(graph.mutations.load(Ordering::SeqCst), 0);
    assert_eq!(correction_callbacks.load(Ordering::SeqCst), 0);
    assert_eq!(event_callbacks.load(Ordering::SeqCst), 0);
    assert_eq!(
        inner_voice
            .try_lock()
            .expect("inner voice lock")
            .corrections_received,
        0
    );
}
