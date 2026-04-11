//! `MockProvider` — test double for LLM providers (REQ-FOR-D6).
//!
//! Records every `complete()` call into a shared `Vec<ProviderCall>`
//! so assertions can inspect prompt history after the system under
//! test finishes running. Also exposes `spawn_mock_server()` which
//! returns a `wiremock::MockServer` pre-stubbed with a default
//! `/v1/chat/completions` responder — phase-7 provider tests extend
//! this with provider-specific routes.
//!
//! This is a *sibling* type, not an impl of any production trait.
//! Phase-7 decides whether to add blanket impls once the provider
//! trait shape is stable.

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// One recorded call to [`MockProvider::complete`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCall {
    pub prompt: String,
}

/// Test double for an LLM provider. Cheap to clone — the call log is
/// shared across clones via `Arc<Mutex<_>>`.
#[derive(Debug, Clone, Default)]
pub struct MockProvider {
    calls: Arc<Mutex<Vec<ProviderCall>>>,
}

impl MockProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the prompt and returns a canned echo response.
    pub async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        self.calls
            .lock()
            .expect("MockProvider call log poisoned")
            .push(ProviderCall {
                prompt: prompt.to_string(),
            });
        Ok(format!("mock:{prompt}"))
    }

    /// Snapshot of every call recorded so far, in order.
    pub fn calls(&self) -> Vec<ProviderCall> {
        self.calls
            .lock()
            .expect("MockProvider call log poisoned")
            .clone()
    }
}

/// Spawns a `wiremock` HTTP server with a default stub for
/// `POST /v1/chat/completions`. Phase-7 provider tests register
/// provider-specific routes on top of this baseline.
pub async fn spawn_mock_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"choices":[{"message":{"content":"mock"}}]}"#,
        ))
        .mount(&server)
        .await;
    server
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_provider_records_calls_in_order() {
        let p = MockProvider::new();
        p.complete("hello").await.unwrap();
        p.complete("world").await.unwrap();
        let calls = p.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].prompt, "hello");
        assert_eq!(calls[1].prompt, "world");
    }
}
