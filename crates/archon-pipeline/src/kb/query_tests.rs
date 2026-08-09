//! Q&A engine tests.
//!
//! Every test runs exact-mode retrieval over a real `archon-docs` corpus with a
//! stub synthesizer, so nothing here needs an embedding provider or a live
//! model and the results are deterministic.

use archon_docs::retrieval::SearchMode;
use cozo::DbInstance;

use super::*;

struct EchoSynthesizer;

#[async_trait::async_trait]
impl QaSynthesizer for EchoSynthesizer {
    async fn synthesize(&self, question: &str, context: &str) -> Result<String> {
        Ok(format!(
            "ANSWER({question}) over {} context char(s)",
            context.len()
        ))
    }
}

/// `Arc` rather than a bare instance: the engine holds the shared handle so the
/// guard-config registration survives (see [`QueryEngine`]).
fn docs_db() -> Arc<DbInstance> {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    Arc::new(db)
}

fn ingest(db: &DbInstance, path: &str, content: &str) -> String {
    archon_docs::ingest_text::ingest_text_source(db, path, "text/plain", content)
        .unwrap()
        .document_id
}

fn exact_options() -> QaQueryOptions {
    QaQueryOptions {
        mode: SearchMode::Exact,
        ..Default::default()
    }
}

#[tokio::test]
async fn a_question_is_answered_from_the_ingested_corpus() {
    let db = docs_db();
    ingest(
        &db,
        "policy.txt",
        "Retention of trading telemetry is thirty days.",
    );
    let engine = QueryEngine::new(db).with_synthesizer(Box::new(EchoSynthesizer));

    let result = engine
        .query("telemetry retention", &exact_options())
        .await
        .unwrap();

    assert!(result.answer.starts_with("ANSWER(telemetry retention)"));
    assert_eq!(result.sources.len(), 1);
    assert!(result.sources[0].quote.contains("thirty days"));
    assert!(result.filed_document_id.is_none());
}

/// REQ-DOCS-015 / REQ-KB-003: an empty corpus must say so rather than invent an
/// answer. The synthesizer is never reached.
#[tokio::test]
async fn an_empty_corpus_reports_insufficient_evidence_instead_of_answering() {
    let db = docs_db();
    let engine = QueryEngine::new(db).with_synthesizer(Box::new(EchoSynthesizer));

    let result = engine.query("anything", &exact_options()).await.unwrap();

    assert!(result.answer.contains("Insufficient context"));
    assert!(result.sources.is_empty());
}

#[tokio::test]
async fn without_a_synthesizer_the_answer_falls_back_to_the_retrieved_context() {
    let db = docs_db();
    ingest(&db, "policy.txt", "Retention is thirty days.");
    let engine = QueryEngine::new(db);

    let result = engine.query("retention", &exact_options()).await.unwrap();

    assert!(result.answer.contains("knowledge base source(s)"));
    assert!(result.answer.contains("thirty days"));
}

#[tokio::test]
async fn a_filed_answer_becomes_a_searchable_document_with_provenance() {
    let db = docs_db();
    ingest(&db, "policy.txt", "Retention is thirty days.");
    let engine = QueryEngine::new(Arc::clone(&db)).with_synthesizer(Box::new(EchoSynthesizer));

    let result = engine
        .query(
            "retention",
            &QaQueryOptions {
                file_answer: true,
                ..exact_options()
            },
        )
        .await
        .unwrap();

    let filed = result.filed_document_id.expect("answer filed");
    let document = archon_docs::store::get_doc_source(&db, &filed)
        .unwrap()
        .unwrap();
    assert!(document.source_path.starts_with(ANSWER_SOURCE_PREFIX));

    // The answer is retrievable as an ordinary document.
    let chunks = archon_docs::store::list_chunks_for_doc(&db, &filed).unwrap();
    assert!(
        chunks
            .iter()
            .any(|c| c.content.contains("ANSWER(retention)"))
    );

    // ...and carries DerivedFrom edges to every chunk it cited.
    let edges = archon_docs::store::list_provenance_from(&db, &filed).unwrap();
    let cited = result.sources[0].chunk_id.clone();
    assert!(
        edges.iter().any(|edge| edge.to_artifact_id == cited
            && matches!(
                edge.edge_type,
                archon_docs::models::ProvenanceEdgeType::DerivedFrom
            )),
        "{edges:?}"
    );
}

/// EC-PIPE-018: a filed answer is second-hand evidence, so identical content
/// scores lower when it lives in an answer document than in a source document.
///
/// Note what this does *not* claim. The penalty is a multiplier, not an
/// ordering guarantee: an answer that restates the question and lists its
/// citations can contain more query terms than the source it came from and
/// still rank above it. The invariant is "lower at equal relevance", which is
/// what the two databases below isolate — same text, same query, different
/// source path.
#[tokio::test]
async fn identical_content_scores_lower_inside_a_filed_answer() {
    const TEXT: &str = "Retention is thirty days.";

    let source_db = docs_db();
    ingest(&source_db, "policy.txt", TEXT);
    let (source_hits, _) = QueryEngine::new(source_db)
        .retrieve("retention", &exact_options())
        .unwrap();

    let answer_db = docs_db();
    ingest(&answer_db, &format!("{ANSWER_SOURCE_PREFIX}a1"), TEXT);
    let (answer_hits, _) = QueryEngine::new(answer_db)
        .retrieve("retention", &exact_options())
        .unwrap();

    assert_eq!(source_hits.len(), 1);
    assert_eq!(answer_hits.len(), 1);
    assert!(
        answer_hits[0].score < source_hits[0].score,
        "answer {} was not penalised against source {}",
        answer_hits[0].score,
        source_hits[0].score
    );
    assert!((answer_hits[0].score - source_hits[0].score * 0.9).abs() < 1e-9);
}

#[tokio::test]
async fn a_knowledge_base_filter_excludes_documents_outside_it() {
    let db = docs_db();
    let inside = ingest(&db, "inside.txt", "Retention is thirty days.");
    ingest(&db, "outside.txt", "Retention is ninety days.");
    archon_docs::store::assign_document_to_kb(&db, "team", &inside).unwrap();
    let engine = QueryEngine::new(db).with_synthesizer(Box::new(EchoSynthesizer));

    let result = engine
        .query(
            "retention",
            &QaQueryOptions {
                kb: Some("team".into()),
                ..exact_options()
            },
        )
        .await
        .unwrap();

    assert_eq!(result.sources.len(), 1);
    assert_eq!(result.sources[0].document_id, inside);
}

/// Filed under `--kb x`, findable under `--kb x`.
#[tokio::test]
async fn a_filed_answer_joins_the_knowledge_base_it_was_scoped_to() {
    let db = docs_db();
    let inside = ingest(&db, "inside.txt", "Retention is thirty days.");
    archon_docs::store::assign_document_to_kb(&db, "team", &inside).unwrap();
    let engine = QueryEngine::new(Arc::clone(&db)).with_synthesizer(Box::new(EchoSynthesizer));

    let result = engine
        .query(
            "retention",
            &QaQueryOptions {
                file_answer: true,
                kb: Some("team".into()),
                ..exact_options()
            },
        )
        .await
        .unwrap();

    let filed = result.filed_document_id.expect("answer filed");
    let members = archon_docs::store::list_kb_document_ids(&db, "team").unwrap();
    assert!(members.contains(&filed), "{members:?}");
}

/// The payoff for running `docs compile` first: a document-level summary the
/// chunk retriever did not surface still reaches the synthesizer.
#[tokio::test]
async fn compiled_summaries_of_a_cited_document_are_added_to_the_context() {
    let db = docs_db();
    let source = ingest(&db, "policy.txt", "Retention is thirty days.");
    let summary = ingest(
        &db,
        &format!("{}{source}", super::super::compile::SUMMARY_SOURCE_PREFIX),
        "This policy fixes the telemetry retention window.",
    );
    archon_docs::store::insert_provenance_edge(
        &db,
        &archon_docs::models::ProvenanceEdge {
            edge_id: "edge-test".into(),
            from_artifact_id: summary,
            to_artifact_id: source,
            edge_type: archon_docs::models::ProvenanceEdgeType::DerivedFrom,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();
    let engine = QueryEngine::new(db);

    let (chunks, _) = engine.retrieve("retention", &exact_options()).unwrap();
    let context = engine.gather_context(chunks, true).unwrap();

    assert_eq!(context.summaries.len(), 1);
    assert!(context.summaries[0].contains("telemetry retention window"));
}

#[tokio::test]
async fn derived_context_can_be_switched_off() {
    let db = docs_db();
    ingest(&db, "policy.txt", "Retention is thirty days.");
    let engine = QueryEngine::new(db);

    let (chunks, _) = engine.retrieve("retention", &exact_options()).unwrap();
    let context = engine.gather_context(chunks, false).unwrap();

    assert!(context.summaries.is_empty());
    assert!(context.concepts.is_empty());
}

#[test]
fn a_long_question_is_truncated_on_character_boundaries() {
    let question = "é".repeat(200);
    let truncated = truncate_chars(&question, 100);
    assert_eq!(truncated.chars().count(), 103); // 100 + "..."
    assert!(truncated.ends_with("..."));
}
