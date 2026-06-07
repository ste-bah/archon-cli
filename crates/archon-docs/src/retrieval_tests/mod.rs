use cozo::DbInstance;

use crate::embed::LocalEmbeddingProvider;
use crate::errors::DocsError;
use crate::models::ChunkArtifact;
use crate::store;

mod exact;
mod hybrid;
mod indexing;

fn test_db() -> DbInstance {
    let path = format!("/tmp/test-retrieval-{}.db", uuid::Uuid::new_v4());
    DbInstance::new("sqlite", &path, "").unwrap()
}

struct MockProvider {
    dim: usize,
}

impl LocalEmbeddingProvider for MockProvider {
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
                normalise(v)
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

struct SynonymProvider;

impl LocalEmbeddingProvider for SynonymProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        Ok(chunks.iter().map(|text| fixture_vector(text)).collect())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        Ok(fixture_vector(query))
    }

    fn dimension(&self) -> usize {
        4
    }

    fn backend_name(&self) -> &'static str {
        "mock-synonym"
    }
}

struct FailingQueryProvider;

impl LocalEmbeddingProvider for FailingQueryProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        Ok(chunks.iter().map(|text| fixture_vector(text)).collect())
    }

    fn embed_query(&self, _query: &str) -> Result<Vec<f32>, DocsError> {
        Err(DocsError::Embedding {
            message: "synthetic query embedding failure".into(),
        })
    }

    fn dimension(&self) -> usize {
        4
    }

    fn backend_name(&self) -> &'static str {
        "mock-query-failure"
    }
}

struct SelectiveMockProvider {
    dim: usize,
}

impl LocalEmbeddingProvider for SelectiveMockProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        chunks
            .iter()
            .map(|c| {
                if c.contains("FAIL") {
                    Err(DocsError::Embedding {
                        message: "simulated embed failure".into(),
                    })
                } else {
                    let mut v = vec![0.0_f32; self.dim];
                    for (j, b) in c.bytes().enumerate() {
                        v[j % self.dim] = (b as f32) / 255.0;
                    }
                    Ok(normalise(v))
                }
            })
            .collect()
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        let chunks = vec![query.to_string()];
        let results = self.embed_chunks(&chunks)?;
        Ok(results.into_iter().next().unwrap_or(vec![0.0; self.dim]))
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn backend_name(&self) -> &'static str {
        "selective-mock"
    }
}

fn setup_with_provider(db: &DbInstance, dim: usize) {
    crate::schema::ensure_doc_schema(db).unwrap();
    crate::schema::ensure_vec_schema(db, dim).unwrap();
    crate::embed::set_provider(Box::new(MockProvider { dim }));
}

fn insert_test_chunk(db: &DbInstance, chunk_id: &str, content: &str) -> ChunkArtifact {
    let chunk = ChunkArtifact {
        chunk_id: chunk_id.into(),
        document_id: format!("doc-{chunk_id}"),
        artifact_id: format!("art-{chunk_id}"),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: content.into(),
        content_hash: format!("hash-{chunk_id}"),
        embedding_status: "pending".into(),
    };
    store::insert_chunk(db, &chunk).unwrap();
    chunk
}

fn fixture_vector(text: &str) -> Vec<f32> {
    let lower = text.to_lowercase();
    let raw = if lower.contains("exact_only") {
        vec![-1.0, 0.0, 0.0, 0.0]
    } else if lower.contains("hybrid_target") {
        vec![0.8, 0.6, 0.0, 0.0]
    } else if lower.contains("car")
        || lower.contains("automobile")
        || lower.contains("vehicle")
        || lower.contains("market signal")
        || lower.contains("pure_semantic")
    {
        vec![1.0, 0.0, 0.0, 0.0]
    } else {
        vec![0.0, 1.0, 0.0, 0.0]
    };
    normalise(raw)
}

fn normalise(mut raw: Vec<f32>) -> Vec<f32> {
    let norm = raw.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    raw.iter_mut().for_each(|x| *x /= norm);
    raw
}
