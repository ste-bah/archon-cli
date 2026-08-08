use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::Value;

use crate::embed::LocalEmbeddingProvider;
use crate::errors::DocsError;

const DEFAULT_MODEL: &str = "text-embedding-3-small";
const MAX_BATCH_SIZE: usize = 256;
const MAX_CHARS_PER_TEXT: usize = 32_764;
const MAX_RETRIES: u32 = 3;

pub struct OpenAiCompatEmbeddingProvider {
    client: Client,
    api_key: String,
    endpoint: String,
    model: String,
}

impl OpenAiCompatEmbeddingProvider {
    pub fn new(
        api_key: String,
        base_url: Option<String>,
        model: Option<String>,
        timeout: Duration,
    ) -> Result<Self, DocsError> {
        if api_key.trim().is_empty() {
            return Err(DocsError::Embedding {
                message: "OpenAI-compatible docs embedding key is empty".into(),
            });
        }
        let endpoint = embedding_endpoint(base_url);
        let client = build_blocking_client(timeout)?;
        Ok(Self {
            client,
            api_key,
            endpoint,
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
        })
    }

    /// Embed one batch, hopping off the async runtime if there is one.
    fn request_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        off_runtime(|| self.request_batch_inner(texts))
    }

    fn request_batch_inner(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        let body = serde_json::json!({
            "model": self.model,
            "input": truncate_texts(texts),
        });

        let mut last_error = None;
        for attempt in 0..MAX_RETRIES {
            let response = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send();
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    last_error = Some(format!("request failed: {error}"));
                    backoff(attempt);
                    continue;
                }
            };
            let status = response.status();
            if status.as_u16() == 401 {
                return Err(DocsError::Embedding {
                    message: "OpenAI-compatible docs embedding key was rejected".into(),
                });
            }
            if status.as_u16() == 429 || status.is_server_error() {
                last_error = Some(format!("provider returned {status}"));
                backoff(attempt);
                continue;
            }
            if !status.is_success() {
                let body = response.text().unwrap_or_default();
                return Err(DocsError::Embedding {
                    message: format!("OpenAI-compatible embedding error {status}: {body}"),
                });
            }
            let value: Value = response.json().map_err(|e| DocsError::Embedding {
                message: format!("failed to parse embedding response: {e}"),
            })?;
            return parse_embeddings(&value, texts.len());
        }

        Err(DocsError::Embedding {
            message: format!(
                "OpenAI-compatible embedding failed after retries: {}",
                last_error.unwrap_or_else(|| "unknown error".into())
            ),
        })
    }
}

impl LocalEmbeddingProvider for OpenAiCompatEmbeddingProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        embed_batches(self, chunks)
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        let mut vectors = self.request_batch(&[query.to_string()])?;
        vectors.pop().ok_or_else(|| DocsError::Embedding {
            message: "embedding provider returned no query vector".into(),
        })
    }

    fn dimension(&self) -> usize {
        if self.model.contains("3-large") {
            3072
        } else {
            1536
        }
    }

    fn backend_name(&self) -> &'static str {
        "openai-compatible"
    }

    fn embedding_space_id(&self) -> String {
        format!("{}:{}:{}", self.backend_name(), self.endpoint, self.model)
    }

    fn max_embedding_workers(&self) -> usize {
        32
    }
}

fn embed_batches(
    provider: &OpenAiCompatEmbeddingProvider,
    chunks: &[String],
) -> Result<Vec<Vec<f32>>, DocsError> {
    let mut all = Vec::with_capacity(chunks.len());
    for batch in chunks.chunks(MAX_BATCH_SIZE) {
        all.extend(provider.request_batch(batch)?);
    }
    Ok(all)
}

/// Build the blocking HTTP client on a thread that is not inside a Tokio
/// runtime.
///
/// `reqwest::blocking` creates and owns a private runtime, and creating one
/// from inside another runtime's context is a documented misuse: the inner
/// runtime's drop tries to block, the outer context forbids blocking, and the
/// process aborts with
///
/// ```text
/// Cannot drop a runtime in a context where blocking is not allowed.
/// This happens when a runtime is dropped from within an asynchronous context.
/// ```
///
/// Every caller reaches this from `#[tokio::main]` — `docs ingest`, `docs
/// index`, `kb`, `reprocess`, `vector-migrate` and the web chat bridge all sit
/// inside the runtime — so with an OpenAI-compatible embedding provider
/// configured, `archon docs ingest` panicked before reading a single byte of
/// the document. Handing the construction to a plain `std::thread` gives it a
/// context with no runtime, which is the one thing `reqwest::blocking` asks
/// for; the client itself is `Send` and works normally afterwards.
fn build_blocking_client(timeout: Duration) -> Result<Client, DocsError> {
    std::thread::scope(|scope| {
        scope
            .spawn(move || Client::builder().timeout(timeout).build())
            .join()
            .map_err(|_| DocsError::Embedding {
                message: "OpenAI-compatible embedding client init thread panicked".into(),
            })?
            .map_err(|error| DocsError::Embedding {
                message: format!("OpenAI-compatible embedding client init failed: {error}"),
            })
    })
}

/// Run a blocking request off the async runtime, for the same reason as
/// [`build_blocking_client`].
///
/// `reqwest::blocking`'s `send` also refuses to run inside a runtime context,
/// so a provider constructed successfully would still fail on first use when
/// the caller is async. Only pay for the thread hop when there actually is a
/// runtime: the indexing path already runs on worker threads, and that is the
/// hot path.
fn off_runtime<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    if tokio::runtime::Handle::try_current().is_err() {
        return work();
    }
    std::thread::scope(|scope| scope.spawn(work).join())
        .unwrap_or_else(|_| panic!("embedding request thread panicked"))
}

fn embedding_endpoint(base_url: Option<String>) -> String {
    let base = base_url
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://api.openai.com/v1".into());
    let base = base.trim_end_matches('/');
    if base.ends_with("/embeddings") {
        base.into()
    } else {
        format!("{base}/embeddings")
    }
}

fn truncate_texts(texts: &[String]) -> Vec<String> {
    texts
        .iter()
        .map(|text| {
            if text.len() <= MAX_CHARS_PER_TEXT {
                return text.clone();
            }
            let mut end = MAX_CHARS_PER_TEXT;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text[..end].to_string()
        })
        .collect()
}

fn parse_embeddings(value: &Value, expected_count: usize) -> Result<Vec<Vec<f32>>, DocsError> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| DocsError::Embedding {
            message: "embedding response missing data array".into(),
        })?;
    if data.len() != expected_count {
        return Err(DocsError::Embedding {
            message: format!("expected {expected_count} embeddings, got {}", data.len()),
        });
    }

    let mut indexed = Vec::with_capacity(expected_count);
    for item in data {
        let index = item.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let embedding = item
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or_else(|| DocsError::Embedding {
                message: "embedding response item missing embedding array".into(),
            })?
            .iter()
            .map(|value| value.as_f64().unwrap_or(0.0) as f32)
            .collect::<Vec<_>>();
        indexed.push((index, super::embed::normalise(&embedding)));
    }
    indexed.sort_by_key(|(index, _)| *index);
    Ok(indexed.into_iter().map(|(_, vector)| vector).collect())
}

fn backoff(attempt: u32) {
    let ms = (100u64 << attempt).min(3200);
    std::thread::sleep(Duration::from_millis(ms));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_space_identity_includes_endpoint_and_model() {
        let first = OpenAiCompatEmbeddingProvider::new(
            "test-key".into(),
            Some("https://first.example/v1".into()),
            Some("model-a".into()),
            Duration::from_secs(1),
        )
        .unwrap();
        let second = OpenAiCompatEmbeddingProvider::new(
            "test-key".into(),
            Some("https://second.example/v1".into()),
            Some("model-b".into()),
            Duration::from_secs(1),
        )
        .unwrap();

        assert_ne!(first.embedding_space_id(), second.embedding_space_id());
    }
}
