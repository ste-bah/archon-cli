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
        let client =
            Client::builder()
                .timeout(timeout)
                .build()
                .map_err(|e| DocsError::Embedding {
                    message: format!("OpenAI-compatible embedding client init failed: {e}"),
                })?;
        Ok(Self {
            client,
            api_key,
            endpoint,
            model: model.unwrap_or_else(|| DEFAULT_MODEL.into()),
        })
    }

    fn request_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
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
