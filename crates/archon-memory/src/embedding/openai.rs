//! OpenAI embedding provider using text-embedding-3-small (1536-dim).

use super::EmbeddingProvider;
use crate::types::MemoryError;

/// Maximum characters per text input (approximate token limit: 8191 tokens * ~4 chars).
const MAX_CHARS_PER_TEXT: usize = 32_764;

/// Maximum texts per API batch request.
const MAX_BATCH_SIZE: usize = 256;

/// Number of retry attempts for transient errors (429, 5xx).
const MAX_RETRIES: u32 = 3;

/// OpenAI text-embedding-3-small provider.
pub struct OpenAIEmbedding {
    api_key: String,
    client: reqwest::blocking::Client,
}

impl OpenAIEmbedding {
    /// Create a new OpenAI embedding provider.
    ///
    /// Returns an error if the API key is empty.
    /// Create a new OpenAI embedding provider.
    ///
    /// Uses `block_in_place` to safely create the reqwest blocking client
    /// even when called from within a tokio async runtime.
    pub fn new(api_key: &str) -> Result<Self, MemoryError> {
        if api_key.is_empty() {
            return Err(MemoryError::Database("OpenAI API key is empty".into()));
        }
        // reqwest::blocking::Client creates its own tokio runtime internally.
        // If we're already inside a tokio runtime, this panics unless we use
        // block_in_place to signal that blocking is intentional.
        let client = tokio::task::block_in_place(|| {
            reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| MemoryError::Database(format!("HTTP client init failed: {e}")))
        })?;
        Ok(Self {
            api_key: api_key.to_string(),
            client,
        })
    }

    /// Send a single batch to the OpenAI embeddings API.
    fn request_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        let truncated: Vec<String> = texts
            .iter()
            .map(|t| {
                if t.len() > MAX_CHARS_PER_TEXT {
                    // Safe UTF-8 truncation: walk back from limit to find char boundary
                    let mut end = MAX_CHARS_PER_TEXT;
                    while end > 0 && !t.is_char_boundary(end) {
                        end -= 1;
                    }
                    t[..end].to_string()
                } else {
                    t.clone()
                }
            })
            .collect();

        let body = serde_json::json!({
            "model": "text-embedding-3-small",
            "input": truncated,
        });

        let mut last_err: Option<MemoryError> = None;
        for attempt in 0..MAX_RETRIES {
            let resp = self
                .client
                .post("https://api.openai.com/v1/embeddings")
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send();

            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(MemoryError::Database(format!(
                        "OpenAI request failed (attempt {}/{}): {e}",
                        attempt + 1,
                        MAX_RETRIES
                    )));
                    backoff(attempt);
                    continue;
                }
            };

            let status = resp.status().as_u16();

            // 401 Unauthorized: don't retry, fail immediately
            if status == 401 {
                return Err(MemoryError::Database(
                    "OpenAI API key is invalid (401 Unauthorized)".into(),
                ));
            }

            // 429 or 5xx: retry with backoff
            if status == 429 || status >= 500 {
                last_err = Some(MemoryError::Database(format!(
                    "OpenAI returned status {status} (attempt {}/{})",
                    attempt + 1,
                    MAX_RETRIES
                )));
                backoff(attempt);
                continue;
            }

            // Parse successful response
            if !(200..300).contains(&status) {
                let text = resp.text().unwrap_or_default();
                return Err(MemoryError::Database(format!(
                    "OpenAI API error {status}: {text}"
                )));
            }

            let parsed: serde_json::Value = resp.json().map_err(|e| {
                MemoryError::Database(format!("failed to parse OpenAI response: {e}"))
            })?;

            return parse_embeddings_response(&parsed, texts.len());
        }

        Err(last_err.unwrap_or_else(|| {
            MemoryError::Database("OpenAI embedding request failed after retries".into())
        }))
    }
}

/// Parse the embeddings array from the OpenAI API response.
fn parse_embeddings_response(
    value: &serde_json::Value,
    expected_count: usize,
) -> Result<Vec<Vec<f32>>, MemoryError> {
    let data = value
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| MemoryError::Database("missing 'data' array in response".into()))?;

    if data.len() != expected_count {
        return Err(MemoryError::Database(format!(
            "expected {} embeddings, got {}",
            expected_count,
            data.len()
        )));
    }

    // OpenAI returns data sorted by index, but let's be safe
    let mut indexed: Vec<(usize, Vec<f32>)> = Vec::with_capacity(expected_count);
    for item in data {
        let index = item
            .get("index")
            .and_then(|i| i.as_u64())
            .ok_or_else(|| MemoryError::Database("missing 'index' in embedding".into()))?
            as usize;

        let embedding = item
            .get("embedding")
            .and_then(|e| e.as_array())
            .ok_or_else(|| MemoryError::Database("missing 'embedding' array".into()))?
            .iter()
            .map(|v| v.as_f64().unwrap_or(0.0) as f32)
            .collect::<Vec<f32>>();

        indexed.push((index, embedding));
    }
    indexed.sort_by_key(|(i, _)| *i);

    Ok(indexed.into_iter().map(|(_, v)| v).collect())
}

/// Simple exponential backoff: 2^attempt * 100ms, capped at 3.2s.
fn backoff(attempt: u32) {
    let ms = (100u64 << attempt).min(3200);
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

impl EmbeddingProvider for OpenAIEmbedding {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Wrap in block_in_place: request_batch calls reqwest::blocking::Client::send()
        // which internally creates a tokio runtime. Without this wrap, that panics
        // with "Cannot start a runtime from within a runtime" when embed() is called
        // from an async tokio task (the normal agent dispatch path).
        tokio::task::block_in_place(|| {
            let mut all_embeddings: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
            for chunk in texts.chunks(MAX_BATCH_SIZE) {
                let batch = self.request_batch(chunk)?;
                all_embeddings.extend(batch);
            }
            Ok(all_embeddings)
        })
    }

    fn dimensions(&self) -> usize {
        1536
    }
}
