//! Tests for KB LLM compilation (REQ-KB-002 / TASK-PIPE-D09).
//!
//! Validates: no-op on an empty corpus, summary generation, concept extraction,
//! provenance edges, incremental compilation, LLM failure resilience, index
//! refresh, `--kb` scoping and `CompileMetrics` accuracy.
//!
//! Every test drives a real `archon-docs` corpus with a stub `KbLlmClient`, so
//! nothing here needs a live model or an embedding provider.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use cozo::DbInstance;

use archon_pipeline::kb::compile::{
    CONCEPT_SOURCE_PREFIX, Compiler, INDEX_SOURCE_PATH, KbLlmClient, SUMMARY_SOURCE_PREFIX,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn docs_db() -> Arc<DbInstance> {
    let db = DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB");
    archon_docs::schema::ensure_doc_schema(&db).expect("docs schema");
    Arc::new(db)
}

/// Ingest through the shipping text path — the same call `archon kb ingest`
/// reaches for a URL or text source.
fn ingest(db: &DbInstance, path: &str, content: &str) -> String {
    archon_docs::ingest_text::ingest_text_source(db, path, "text/plain", content)
        .expect("ingest")
        .document_id
}

fn documents_with_prefix(db: &DbInstance, prefix: &str) -> Vec<String> {
    archon_docs::store::list_doc_sources(db)
        .expect("list doc sources")
        .into_iter()
        .filter(|doc| doc.source_path.starts_with(prefix))
        .map(|doc| doc.document_id)
        .collect()
}

fn document_text(db: &DbInstance, document_id: &str) -> String {
    let mut chunks = archon_docs::store::list_chunks_for_doc(db, document_id).expect("chunks");
    chunks.sort_by_key(|chunk| chunk.chunk_index);
    chunks
        .iter()
        .map(|chunk| chunk.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

// ---------------------------------------------------------------------------
// Stub LLM clients
// ---------------------------------------------------------------------------

/// Returns a well-formed response for each of the three prompt kinds.
struct MockSummaryLlm;

#[async_trait::async_trait]
impl KbLlmClient for MockSummaryLlm {
    async fn complete(&self, prompt: &str) -> Result<String> {
        if prompt.starts_with("Extract key concepts") {
            Ok(
                r#"[{"name": "TestConcept", "explanation": "A concept for testing.", "source_documents": []}]"#
                    .to_string(),
            )
        } else if prompt.starts_with("Given these concepts") {
            Ok("[]".to_string())
        } else {
            Ok(r#"{"summary": "This is a test summary."}"#.to_string())
        }
    }
}

/// Cites every document it is shown, so concept provenance can be asserted.
struct CitingConceptLlm;

#[async_trait::async_trait]
impl KbLlmClient for CitingConceptLlm {
    async fn complete(&self, prompt: &str) -> Result<String> {
        if prompt.starts_with("Extract key concepts") {
            let ids: Vec<String> = prompt
                .lines()
                .filter_map(|line| line.split("(ID: ").nth(1))
                .filter_map(|rest| rest.split(')').next())
                .map(|id| format!("\"{id}\""))
                .collect();
            Ok(format!(
                r#"[{{"name": "Alpha", "explanation": "First.", "source_documents": [{}]}},
                    {{"name": "Beta", "explanation": "Second.", "source_documents": [{}]}}]"#,
                ids.join(", "),
                ids.join(", ")
            ))
        } else if prompt.starts_with("Given these concepts") {
            Ok(
                r#"[{"source": "Alpha", "target": "Beta", "relationship": "relates to"}]"#
                    .to_string(),
            )
        } else {
            Ok(r#"{"summary": "Cited summary."}"#.to_string())
        }
    }
}

/// Wraps its JSON in a markdown fence, the way real providers do.
struct FencedJsonLlm;

#[async_trait::async_trait]
impl KbLlmClient for FencedJsonLlm {
    async fn complete(&self, prompt: &str) -> Result<String> {
        let inner = MockSummaryLlm.complete(prompt).await?;
        Ok(format!("Here you go:\n\n```json\n{inner}\n```\n"))
    }
}

/// Never returns JSON.
struct BadJsonLlm;

#[async_trait::async_trait]
impl KbLlmClient for BadJsonLlm {
    async fn complete(&self, _prompt: &str) -> Result<String> {
        Ok("not valid json at all".to_string())
    }
}

/// Fails every call.
struct FailingLlm;

#[async_trait::async_trait]
impl KbLlmClient for FailingLlm {
    async fn complete(&self, _prompt: &str) -> Result<String> {
        anyhow::bail!("provider unavailable")
    }
}

/// Records the prompts it was sent.
struct RecordingLlm {
    prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl KbLlmClient for RecordingLlm {
    async fn complete(&self, prompt: &str) -> Result<String> {
        self.prompts.lock().unwrap().push(prompt.to_string());
        MockSummaryLlm.complete(prompt).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_empty_corpus_compiles_to_zero_without_calling_the_model() {
    let db = docs_db();
    let compiler = Compiler::new(db, Box::new(FailingLlm)).expect("compiler");

    let metrics = compiler.compile().await.expect("compile");

    assert_eq!(metrics.summaries_generated, 0);
    assert_eq!(metrics.concepts_extracted, 0);
    assert_eq!(metrics.edges_created, 0);
    assert!(!metrics.index_updated);
}

/// The headline acceptance: content ingested the shipping way is compiled, and
/// the output lands in the same store the search commands read.
#[tokio::test]
async fn documents_from_the_shipping_ingest_path_are_summarised_into_the_same_store() {
    let db = docs_db();
    let source = ingest(&db, "policy.txt", "Retention of telemetry is thirty days.");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(MockSummaryLlm)).expect("compiler");

    let metrics = compiler.compile().await.expect("compile");

    assert_eq!(metrics.summaries_generated, 1);
    let summaries = documents_with_prefix(&db, SUMMARY_SOURCE_PREFIX);
    assert_eq!(summaries.len(), 1);
    assert!(document_text(&db, &summaries[0]).contains("This is a test summary."));

    // The summary is linked back to the document it came from.
    let edges = archon_docs::store::list_provenance_from(&db, &summaries[0]).expect("edges");
    assert!(edges.iter().any(|edge| edge.to_artifact_id == source));
}

#[tokio::test]
async fn concepts_are_stored_as_documents_and_linked_to_their_sources() {
    let db = docs_db();
    let first = ingest(&db, "a.txt", "Alpha content about retention.");
    let second = ingest(&db, "b.txt", "Beta content about retention.");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(CitingConceptLlm)).expect("compiler");

    let metrics = compiler.compile().await.expect("compile");

    assert_eq!(metrics.concepts_extracted, 2);
    let concepts = documents_with_prefix(&db, CONCEPT_SOURCE_PREFIX);
    assert_eq!(concepts.len(), 2);
    for concept in &concepts {
        let targets: Vec<String> = archon_docs::store::list_provenance_from(&db, concept)
            .expect("edges")
            .into_iter()
            .map(|edge| edge.to_artifact_id)
            .collect();
        assert!(targets.contains(&first), "{targets:?}");
        assert!(targets.contains(&second), "{targets:?}");
    }
}

#[tokio::test]
async fn cross_references_link_concept_documents_to_each_other() {
    let db = docs_db();
    ingest(&db, "a.txt", "Alpha content.");
    ingest(&db, "b.txt", "Beta content.");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(CitingConceptLlm)).expect("compiler");

    compiler.compile().await.expect("compile");

    let concepts = documents_with_prefix(&db, CONCEPT_SOURCE_PREFIX);
    let cites: Vec<_> = concepts
        .iter()
        .flat_map(|c| archon_docs::store::list_provenance_from(&db, c).expect("edges"))
        .filter(|edge| {
            matches!(
                edge.edge_type,
                archon_docs::models::ProvenanceEdgeType::Cites
            )
        })
        .collect();
    assert_eq!(cites.len(), 1, "{cites:?}");
    assert!(concepts.contains(&cites[0].to_artifact_id));
}

#[tokio::test]
async fn the_index_document_is_refreshed_and_never_duplicated() {
    let db = docs_db();
    ingest(&db, "a.txt", "Alpha content.");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(MockSummaryLlm)).expect("compiler");
    compiler.compile().await.expect("first compile");

    ingest(&db, "b.txt", "Beta content.");
    let metrics = compiler.compile().await.expect("second compile");

    assert!(metrics.index_updated);
    let indexes = documents_with_prefix(&db, INDEX_SOURCE_PATH);
    assert_eq!(indexes.len(), 1);
    let text = document_text(&db, &indexes[0]);
    assert!(text.contains("Source documents: 2"), "{text}");
    assert!(text.contains("Compiled summaries: 2"), "{text}");
}

/// The watermark: a second run with nothing new does no work, and a third run
/// picks up only what arrived since.
#[tokio::test]
async fn compilation_is_incremental_across_runs() {
    let db = docs_db();
    ingest(&db, "a.txt", "Alpha content.");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(MockSummaryLlm)).expect("compiler");

    let first = compiler.compile().await.expect("first compile");
    assert_eq!(first.summaries_generated, 1);

    let second = compiler.compile().await.expect("second compile");
    assert_eq!(second.summaries_generated, 0);

    ingest(&db, "b.txt", "Beta content.");
    let third = compiler.compile().await.expect("third compile");
    assert_eq!(third.summaries_generated, 1);
}

/// Without this the second run summarises the first run's summaries and the
/// corpus grows without bound.
#[tokio::test]
async fn the_pass_never_compiles_its_own_output() {
    let db = docs_db();
    ingest(&db, "a.txt", "Alpha content.");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(MockSummaryLlm)).expect("compiler");
    compiler.compile().await.expect("first compile");

    // Reset the watermark so every document is a candidate again; only the
    // derived-source filter can keep the summaries out now.
    let reset = Compiler::new(Arc::clone(&db), Box::new(MockSummaryLlm)).expect("compiler");
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let recording = Compiler::new(
        Arc::clone(&db),
        Box::new(RecordingLlm {
            prompts: Arc::clone(&prompts),
        }),
    )
    .expect("compiler");
    drop(reset);
    recording.compile().await.expect("re-compile");

    let seen = prompts.lock().unwrap().clone();
    assert!(
        !seen.iter().any(|p| p.contains("This is a test summary.")),
        "a summary was fed back into the compiler: {seen:?}"
    );
}

/// Regression: the first live run stored the fence markup as the summary and
/// extracted zero concepts, because every parse hit the ``` prefix.
#[tokio::test]
async fn json_wrapped_in_a_markdown_fence_is_still_parsed() {
    let db = docs_db();
    ingest(&db, "a.txt", "Alpha content.");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(FencedJsonLlm)).expect("compiler");

    let metrics = compiler.compile().await.expect("compile");

    assert_eq!(metrics.summaries_generated, 1);
    assert_eq!(metrics.concepts_extracted, 1);
    let summaries = documents_with_prefix(&db, SUMMARY_SOURCE_PREFIX);
    let text = document_text(&db, &summaries[0]);
    assert!(text.contains("This is a test summary."), "{text}");
    assert!(
        !text.contains("```"),
        "fence markup leaked into the summary: {text}"
    );
}

#[tokio::test]
async fn a_non_json_response_is_kept_as_the_summary_rather_than_discarded() {
    let db = docs_db();
    ingest(&db, "a.txt", "Alpha content.");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(BadJsonLlm)).expect("compiler");

    let metrics = compiler.compile().await.expect("compile");

    assert_eq!(metrics.summaries_generated, 1);
    let summaries = documents_with_prefix(&db, SUMMARY_SOURCE_PREFIX);
    assert!(document_text(&db, &summaries[0]).contains("not valid json at all"));
}

/// One failing document must not abandon the rest of the batch.
#[tokio::test]
async fn a_provider_failure_is_survivable_and_reported_as_zero_summaries() {
    let db = docs_db();
    ingest(&db, "a.txt", "Alpha content.");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(FailingLlm)).expect("compiler");

    let metrics = compiler.compile().await.expect("compile must not error");

    assert_eq!(metrics.summaries_generated, 0);
    assert!(documents_with_prefix(&db, SUMMARY_SOURCE_PREFIX).is_empty());
}

#[tokio::test]
async fn a_kb_filter_restricts_compilation_and_the_summary_joins_that_kb() {
    let db = docs_db();
    let inside = ingest(&db, "inside.txt", "Inside content.");
    ingest(&db, "outside.txt", "Outside content.");
    archon_docs::store::assign_document_to_kb(&db, "team", &inside).expect("assign");
    let compiler = Compiler::new(Arc::clone(&db), Box::new(MockSummaryLlm))
        .expect("compiler")
        .with_kb(Some("team".into()));

    let metrics = compiler.compile().await.expect("compile");

    assert_eq!(metrics.summaries_generated, 1);
    let members = archon_docs::store::list_kb_document_ids(&db, "team").expect("members");
    let summaries = documents_with_prefix(&db, SUMMARY_SOURCE_PREFIX);
    assert_eq!(summaries.len(), 1);
    assert!(
        members.contains(&summaries[0]),
        "the summary must be searchable under --kb team"
    );
}

#[tokio::test]
async fn progress_is_reported_once_per_document() {
    use archon_pipeline::kb::compile::{CompilePhase, CompileProgress};

    let db = docs_db();
    ingest(&db, "a.txt", "Alpha content.");
    ingest(&db, "b.txt", "Beta content.");
    let seen: Arc<Mutex<Vec<CompileProgress>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let compiler = Compiler::new(db, Box::new(MockSummaryLlm))
        .expect("compiler")
        .with_progress(Arc::new(move |event| sink.lock().unwrap().push(event)));

    compiler.compile().await.expect("compile");

    let events = seen.lock().unwrap();
    let summarized = events
        .iter()
        .filter(|e| e.phase == CompilePhase::DocumentSummarized)
        .count();
    assert_eq!(summarized, 2);
    assert!(events.iter().any(|e| e.phase == CompilePhase::IndexUpdated));
    assert!(
        events
            .iter()
            .any(|e| e.phase == CompilePhase::DocumentsSelected && e.document_total == 2)
    );
}
