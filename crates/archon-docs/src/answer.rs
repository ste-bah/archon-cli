//! Grounded answer synthesis with citation provenance.
//!
//! The answer flow: embed query → HNSW search → assemble answer
//! with inline citations that resolve to source document/page/chunk records.

use cozo::DbInstance;

use crate::errors::DocsError;
use crate::retrieval::{SearchResult, search};

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
            snippet: if r.content.len() > 200 {
                format!("{}...", &r.content[..200])
            } else {
                r.content.clone()
            },
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
            t.push_str(&format!(
                "\n[{}] (pages {}-{}, score {:.2}):\n{}\n",
                i + 1,
                r.page_start,
                r.page_end,
                r.score,
                r.content
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
        crate::store::insert_provenance_edge(db, &edge).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
        inserted += 1;
    }
    Ok(inserted)
}

fn new_answer_id() -> String {
    format!(
        "answer-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
    )
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
