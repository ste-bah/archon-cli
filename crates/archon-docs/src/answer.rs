//! Grounded answer synthesis with citation provenance.
//!
//! The answer flow: embed query → HNSW search → assemble answer
//! with inline citations that resolve to source document/page/chunk records.

use std::thread;
use std::time::Duration;

use cozo::{DataValue, DbInstance, ScriptMutability};

pub use crate::answer_timecode::{
    TimecodeMs, format_citation, format_citation_page, format_citation_video, format_ms_as_timecode,
};
use crate::errors::DocsError;
use crate::retrieval::{SearchResult, search};

const CITATION_SNIPPET_CHARS: usize = 200;
const PROVENANCE_LOCK_RETRY_DELAYS_MS: &[u64] = &[50, 100, 200, 400];

/// An answer with supporting evidence citations.
#[derive(Clone, Debug)]
pub struct Answer {
    pub answer_id: String,
    pub query: String,
    /// The synthesised answer text assembled from retrieved chunks.
    pub text: String,
    /// Citations linking claims to source chunks.
    pub citations: Vec<Citation>,
    /// Raw search results for debug/inspection.
    pub sources: Vec<SearchResult>,
}

/// A citation linking answer text to a specific source chunk.
#[derive(Clone, Debug)]
pub struct Citation {
    pub chunk_id: String,
    pub document_id: String,
    pub page_start: u32,
    pub page_end: u32,
    /// The snippet of source text this citation refers to.
    pub snippet: String,
}

/// Answer a query using retrieval-augmented evidence.
///
/// Returns a structured error if no embedding provider is configured
/// or no documents are indexed.
pub fn answer(db: &DbInstance, query: &str, top_k: usize) -> Result<Answer, DocsError> {
    answer_with_timeref(db, query, top_k, &|chunk_id| {
        lookup_video_timecode(db, chunk_id).unwrap_or(None)
    })
}

pub fn answer_with_timeref(
    db: &DbInstance,
    query: &str,
    top_k: usize,
    timeref_resolver: &dyn Fn(&str) -> Option<TimecodeMs>,
) -> Result<Answer, DocsError> {
    let answer_id = new_answer_id();
    let search_results = match search(db, query, top_k) {
        Ok(results) => results,
        Err(DocsError::Embedding { message }) => {
            return Ok(Answer {
                answer_id,
                query: query.to_string(),
                text: message,
                citations: Vec::new(),
                sources: Vec::new(),
            });
        }
        Err(e) => return Err(e),
    };

    if search_results.results.is_empty() && search_results.total_chunks == 0 {
        return Ok(Answer {
            answer_id,
            query: query.to_string(),
            text: "No documents have been indexed. Ingest documents first using 'archon docs ingest <path>'.".into(),
            citations: Vec::new(),
            sources: Vec::new(),
        });
    }

    if search_results.results.is_empty() {
        return Ok(Answer {
            answer_id,
            query: query.to_string(),
            text: format!(
                "No relevant evidence found for query. {} chunks are stored ({} indexed), but none matched closely enough.",
                search_results.total_chunks, search_results.total_indexed_chunks
            ),
            citations: Vec::new(),
            sources: Vec::new(),
        });
    }

    let citations: Vec<Citation> = search_results
        .results
        .iter()
        .map(|r| Citation {
            chunk_id: r.chunk_id.clone(),
            document_id: r.document_id.clone(),
            page_start: r.page_start,
            page_end: r.page_end,
            snippet: truncate_chars(&r.content, CITATION_SNIPPET_CHARS),
        })
        .collect();

    // Assemble answer text from top sources (Phase 2: extractive; future: LLM)
    let text = if search_results.results.len() == 1 {
        format!(
            "Based on 1 source (score {:.2}):\n\n{}",
            search_results.results[0].score, search_results.results[0].content
        )
    } else {
        let mut t = format!("Based on {} sources:\n", search_results.results.len());
        for (i, r) in search_results.results.iter().enumerate() {
            let citation = Citation {
                chunk_id: r.chunk_id.clone(),
                document_id: r.document_id.clone(),
                page_start: r.page_start,
                page_end: r.page_end,
                snippet: r.content.clone(),
            };
            t.push_str(&format_citation(
                i + 1,
                &citation,
                r.score,
                timeref_resolver(&r.chunk_id),
            ));
        }
        t
    };

    Ok(Answer {
        answer_id,
        query: query.to_string(),
        text,
        citations,
        sources: search_results.results,
    })
}

fn lookup_video_timecode(db: &DbInstance, chunk_id: &str) -> Result<Option<TimecodeMs>, DocsError> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("cid".into(), DataValue::from(chunk_id));
    let result = match db.run_script(
        "?[start] := *video_chunk_timeref{chunk_id, timestamp_start_ms: start}, chunk_id = $cid",
        params,
        ScriptMutability::Immutable,
    ) {
        Ok(result) => result,
        Err(_) => return Ok(None),
    };
    Ok(result.rows.first().map(|row| TimecodeMs {
        start_ms: row[0].get_int().unwrap_or(0),
    }))
}

/// Persist answer -> cited chunk edges so `/docs provenance <answer-id>` can
/// traverse from answer to source chunks/pages/documents.
pub fn persist_answer_provenance(db: &DbInstance, answer: &Answer) -> Result<usize, DocsError> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut inserted = 0;
    for citation in &answer.citations {
        let edge = crate::models::ProvenanceEdge {
            edge_id: format!("edge-{}-{}", answer.answer_id, citation.chunk_id),
            from_artifact_id: answer.answer_id.clone(),
            to_artifact_id: citation.chunk_id.clone(),
            edge_type: crate::models::ProvenanceEdgeType::Cites,
            created_at: now.clone(),
        };
        if insert_answer_provenance_edge(db, &edge)? {
            inserted += 1;
        }
    }
    Ok(inserted)
}

fn insert_answer_provenance_edge(
    db: &DbInstance,
    edge: &crate::models::ProvenanceEdge,
) -> Result<bool, DocsError> {
    for attempt in 0..=PROVENANCE_LOCK_RETRY_DELAYS_MS.len() {
        match crate::store::insert_provenance_edge(db, edge) {
            Ok(()) => return Ok(true),
            Err(e) => {
                let message = e.to_string();
                if !is_answer_provenance_lock_or_busy(&message) {
                    return Err(DocsError::Storage { message });
                }

                if let Some(delay_ms) = PROVENANCE_LOCK_RETRY_DELAYS_MS.get(attempt) {
                    thread::sleep(Duration::from_millis(*delay_ms));
                    continue;
                }

                tracing::warn!(
                    edge_id = %edge.edge_id,
                    error = %message,
                    "skipping answer provenance edge after sqlite lock retries"
                );
                return Ok(false);
            }
        }
    }

    Ok(false)
}

fn new_answer_id() -> String {
    format!(
        "answer-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
    )
}

pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

fn is_answer_provenance_lock_or_busy(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("database is locked")
        || lower.contains("code 5")
        || (lower.contains("doc_provenance_edges")
            && lower.contains("when executing against relation"))
}

/// Verify that every citation resolves to a real record.
///
/// Returns a list of citation indices that failed resolution (empty = all valid).
pub fn verify_citations(db: &DbInstance, citations: &[Citation]) -> Result<Vec<usize>, DocsError> {
    let mut failed = Vec::new();
    for (i, c) in citations.iter().enumerate() {
        let chunks = crate::store::list_chunks_for_doc(db, &c.document_id).map_err(|e| {
            DocsError::Retrieval {
                message: e.to_string(),
            }
        })?;
        let found = chunks.iter().any(|ch| {
            ch.chunk_id == c.chunk_id && ch.page_start == c.page_start && ch.page_end == c.page_end
        });
        if !found {
            failed.push(i);
        }
    }
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-answer-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    struct MockProvider {
        dim: usize,
    }

    impl crate::embed::LocalEmbeddingProvider for MockProvider {
        fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            Ok(chunks
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let mut v = vec![0.0_f32; self.dim];
                    for (j, b) in c.bytes().enumerate() {
                        v[j % self.dim] = (b as f32) / 255.0;
                    }
                    v[0] = (i as f32 + 1.0) * 0.5;
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
                    v.iter_mut().for_each(|x| *x /= norm);
                    v
                })
                .collect())
        }

        fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
            let chunks = vec![query.to_string()];
            let mut results = self.embed_chunks(&chunks)?;
            Ok(results.remove(0))
        }

        fn dimension(&self) -> usize {
            self.dim
        }

        fn backend_name(&self) -> &'static str {
            "mock"
        }
    }

    fn setup(db: &DbInstance) {
        crate::schema::ensure_doc_schema(db).unwrap();
        crate::schema::ensure_vec_schema(db, 4).unwrap();
        crate::embed::set_provider(Box::new(MockProvider { dim: 4 }));
    }

    #[test]
    fn test_answer_empty_corpus() {
        let db = test_db();
        setup(&db);

        let ans = answer(&db, "test query", 5).unwrap();
        assert!(ans.text.contains("No documents have been indexed"));
        assert!(ans.citations.is_empty());
    }

    #[test]
    fn test_answer_with_citations() {
        let db = test_db();
        setup(&db);

        let chunk = crate::models::ChunkArtifact {
            chunk_id: "chunk-ans-1".into(),
            document_id: "doc-ans".into(),
            artifact_id: "art-1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "Quantum computing uses qubits instead of classical bits.".into(),
            content_hash: "hash1".into(),
            embedding_status: "pending".into(),
        };
        crate::store::insert_chunk(&db, &chunk).unwrap();
        crate::retrieval::index_chunk(&db, &chunk).unwrap();

        let ans = answer(&db, "quantum qubits", 5).unwrap();
        assert!(!ans.text.is_empty());
        assert!(!ans.citations.is_empty());
        assert_eq!(ans.citations[0].chunk_id, "chunk-ans-1");
        assert_eq!(ans.citations[0].page_start, 1);
    }

    #[test]
    fn test_answer_citation_snippet_is_unicode_safe() {
        let db = test_db();
        setup(&db);

        let chunk = crate::models::ChunkArtifact {
            chunk_id: "chunk-unicode-1".into(),
            document_id: "doc-unicode".into(),
            artifact_id: "art-unicode-1".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: format!("{} sanctions scoring evidence", "é".repeat(201)),
            content_hash: "hash-unicode-1".into(),
            embedding_status: "pending".into(),
        };
        crate::store::insert_chunk(&db, &chunk).unwrap();
        crate::retrieval::index_chunk(&db, &chunk).unwrap();

        let ans = answer(&db, "sanctions scoring", 5).unwrap();
        assert!(!ans.citations.is_empty());
        assert_eq!(
            ans.citations[0]
                .snippet
                .chars()
                .filter(|c| *c == 'é')
                .count(),
            200
        );
        assert!(ans.citations[0].snippet.ends_with("..."));
    }

    #[test]
    fn test_answer_provenance_edges_are_persisted() {
        let db = test_db();
        setup(&db);

        let chunk = crate::models::ChunkArtifact {
            chunk_id: "chunk-prov-1".into(),
            document_id: "doc-prov".into(),
            artifact_id: "art-prov-1".into(),
            chunk_index: 0,
            page_start: 2,
            page_end: 2,
            content: "Policy marketplaces need citation-backed governance.".into(),
            content_hash: "hash-prov-1".into(),
            embedding_status: "pending".into(),
        };
        crate::store::insert_chunk(&db, &chunk).unwrap();
        crate::retrieval::index_chunk(&db, &chunk).unwrap();

        let ans = answer(&db, "citation backed governance", 5).unwrap();
        assert!(ans.answer_id.starts_with("answer-"));
        let inserted = persist_answer_provenance(&db, &ans).unwrap();
        assert_eq!(inserted, ans.citations.len());

        let edges = crate::store::list_provenance_from(&db, &ans.answer_id).unwrap();
        assert_eq!(edges.len(), ans.citations.len());
        assert_eq!(edges[0].from_artifact_id, ans.answer_id);
        assert_eq!(edges[0].to_artifact_id, "chunk-prov-1");
        assert_eq!(edges[0].edge_type, crate::models::ProvenanceEdgeType::Cites);
    }

    #[test]
    fn test_answer_provenance_lock_detection_matches_cozo_messages() {
        assert!(is_answer_provenance_lock_or_busy(
            "insert provenance edge failed: database is locked (code 5)"
        ));
        assert!(is_answer_provenance_lock_or_busy(
            "insert provenance edge failed: when executing against relation 'doc_provenance_edges'"
        ));
        assert!(!is_answer_provenance_lock_or_busy(
            "insert provenance edge failed: missing parameter"
        ));
    }

    #[test]
    fn test_timecode_formatter_boundaries() {
        assert_eq!(format_ms_as_timecode(760_000), "12:40");
        assert_eq!(format_ms_as_timecode(0), "00:00");
        assert_eq!(format_ms_as_timecode(3_661_000), "01:01:01");
        assert_eq!(format_ms_as_timecode(7_200_000), "02:00:00");
    }

    #[test]
    fn test_answer_citations_resolve() {
        let db = test_db();
        setup(&db);

        let chunk = crate::models::ChunkArtifact {
            chunk_id: "chunk-resolve-1".into(),
            document_id: "doc-resolve".into(),
            artifact_id: "art-1".into(),
            chunk_index: 0,
            page_start: 3,
            page_end: 5,
            content: "Machine learning models require training data.".into(),
            content_hash: "hash2".into(),
            embedding_status: "pending".into(),
        };
        crate::store::insert_chunk(&db, &chunk).unwrap();
        crate::retrieval::index_chunk(&db, &chunk).unwrap();

        let ans = answer(&db, "machine learning training", 5).unwrap();
        assert!(!ans.citations.is_empty());

        let failures = verify_citations(&db, &ans.citations).unwrap();
        assert!(
            failures.is_empty(),
            "all citations must resolve to real records"
        );
    }
}
