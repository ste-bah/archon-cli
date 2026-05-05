use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::json;

use crate::errors::DocsError;
use crate::vlm::mime::detect_mime;
use crate::vlm::{IMAGE_DESCRIPTION_PROMPT, VlmDescriptionProvider};

pub const DEFAULT_GEMINI_ENDPOINT_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-3-flash-preview";

#[derive(Debug)]
pub struct GeminiVlmProvider {
    api_key: String,
    model: String,
    endpoint_base: String,
    http: reqwest::blocking::Client,
    rate_limiter: GeminiRateLimiter,
    retry_base_delay: Duration,
}

impl GeminiVlmProvider {
    pub fn new(
        api_key: impl Into<String>,
        model: impl Into<String>,
        endpoint_base: impl Into<String>,
        rpm_limit: u32,
    ) -> Result<Self, DocsError> {
        Self::new_with_retry_delay(
            api_key,
            model,
            endpoint_base,
            rpm_limit,
            Duration::from_secs(1),
        )
    }

    fn new_with_retry_delay(
        api_key: impl Into<String>,
        model: impl Into<String>,
        endpoint_base: impl Into<String>,
        rpm_limit: u32,
        retry_base_delay: Duration,
    ) -> Result<Self, DocsError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(DocsError::VlmAuthentication {
                provider: "gemini".into(),
                message: "Google API key missing".into(),
            });
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| DocsError::VlmProvider {
                provider: "gemini".into(),
                message: format!("failed to build HTTP client: {e}"),
                status_code: None,
            })?;
        Ok(Self {
            api_key,
            model: model.into(),
            endpoint_base: endpoint_base.into(),
            http,
            rate_limiter: GeminiRateLimiter::new(rpm_limit),
            retry_base_delay,
        })
    }

    pub fn from_policy(
        policy: &archon_policy::GeminiVlmPolicy,
        api_key: impl Into<String>,
    ) -> Result<Self, DocsError> {
        Self::new(
            api_key,
            policy.model.clone(),
            policy.endpoint_base.clone(),
            policy.rpm_limit,
        )
    }

    pub fn provider_id(&self) -> &'static str {
        "gemini"
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn endpoint_base(&self) -> &str {
        &self.endpoint_base
    }

    pub fn health_check(&self) -> Result<(), DocsError> {
        let url = format!(
            "{}/models?key={}",
            self.endpoint_base.trim_end_matches('/'),
            self.api_key
        );
        let response = self.http.get(url).send().map_err(map_send_error)?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(DocsError::VlmAuthentication {
                provider: "gemini".into(),
                message: format!("models endpoint rejected credentials with HTTP {status}"),
            });
        }
        if !status.is_success() {
            return Err(DocsError::VlmProvider {
                provider: "gemini".into(),
                message: format!("health check failed with HTTP {status}"),
                status_code: Some(status.as_u16()),
            });
        }
        let models: ModelsResponse = response.json().map_err(|e| DocsError::VlmProvider {
            provider: "gemini".into(),
            message: format!("failed to parse models response: {e}"),
            status_code: None,
        })?;
        let expected = format!("models/{}", self.model);
        let found = models
            .models
            .iter()
            .any(|model| model.name == self.model || model.name == expected);
        if found {
            Ok(())
        } else {
            Err(DocsError::VlmProvider {
                provider: "gemini".into(),
                message: format!(
                    "model '{}' not returned by Gemini models endpoint",
                    self.model
                ),
                status_code: None,
            })
        }
    }

    fn generate_once(&self, image_bytes: &[u8]) -> Result<String, DocsError> {
        self.rate_limiter.acquire()?;
        let mime = detect_mime(image_bytes)?;
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            self.endpoint_base.trim_end_matches('/'),
            self.model,
            self.api_key
        );
        let body = json!({
            "contents": [{
                "parts": [
                    {"text": IMAGE_DESCRIPTION_PROMPT},
                    {"inlineData": {"mimeType": mime, "data": STANDARD.encode(image_bytes)}}
                ]
            }]
        });
        let response = self
            .http
            .post(url)
            .json(&body)
            .send()
            .map_err(map_send_error)?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(DocsError::VlmRateLimit {
                provider: "gemini".into(),
                retry_after_secs: 1,
                message: "Gemini returned HTTP 429".into(),
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(DocsError::VlmAuthentication {
                provider: "gemini".into(),
                message: format!("generateContent rejected credentials with HTTP {status}"),
            });
        }
        if !status.is_success() {
            return Err(DocsError::VlmProvider {
                provider: "gemini".into(),
                message: format!("generateContent failed with HTTP {status}"),
                status_code: Some(status.as_u16()),
            });
        }
        let parsed: GenerateResponse = response.json().map_err(|e| DocsError::VlmProvider {
            provider: "gemini".into(),
            message: format!("failed to parse generateContent response: {e}"),
            status_code: None,
        })?;
        parsed
            .candidates
            .first()
            .and_then(|candidate| candidate.content.parts.first())
            .and_then(|part| part.text.clone())
            .ok_or_else(|| DocsError::VlmProvider {
                provider: "gemini".into(),
                message: "generateContent response did not contain text".into(),
                status_code: None,
            })
    }
}

impl VlmDescriptionProvider for GeminiVlmProvider {
    fn describe_image(&self, image_bytes: &[u8]) -> Result<String, DocsError> {
        match self.generate_once(image_bytes) {
            Err(DocsError::VlmRateLimit { .. }) => {
                std::thread::sleep(self.retry_base_delay);
                self.generate_once(image_bytes)
            }
            other => other,
        }
    }
}

#[derive(Debug)]
pub struct GeminiRateLimiter {
    rpm_limit: usize,
    window: Duration,
    state: Mutex<VecDeque<Instant>>,
}

impl GeminiRateLimiter {
    pub fn new(rpm_limit: u32) -> Self {
        Self {
            rpm_limit: rpm_limit.max(1) as usize,
            window: Duration::from_secs(60),
            state: Mutex::new(VecDeque::new()),
        }
    }

    pub fn acquire(&self) -> Result<(), DocsError> {
        loop {
            match self.try_acquire_at(Instant::now()) {
                Ok(()) => return Ok(()),
                Err(wait) => std::thread::sleep(wait.min(Duration::from_secs(60))),
            }
        }
    }

    fn try_acquire_at(&self, now: Instant) -> Result<(), Duration> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while state
            .front()
            .map(|oldest| now.duration_since(*oldest) >= self.window)
            .unwrap_or(false)
        {
            state.pop_front();
        }
        if state.len() < self.rpm_limit {
            state.push_back(now);
            return Ok(());
        }
        let oldest = *state.front().expect("non-empty after limit check");
        Err(self
            .window
            .saturating_sub(now.duration_since(oldest))
            .max(Duration::from_millis(1)))
    }
}

fn map_send_error(error: reqwest::Error) -> DocsError {
    if error.is_timeout() {
        DocsError::VlmTimeout {
            provider: "gemini".into(),
            message: error.to_string(),
        }
    } else {
        DocsError::VlmProvider {
            provider: "gemini".into(),
            message: error.to_string(),
            status_code: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    models: Vec<ModelRecord>,
}

#[derive(Debug, Deserialize)]
struct ModelRecord {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GenerateResponse {
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: Content,
}

#[derive(Debug, Deserialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
struct Part {
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(endpoint_base: String) -> GeminiVlmProvider {
        GeminiVlmProvider::new_with_retry_delay(
            "test-key",
            DEFAULT_GEMINI_MODEL,
            endpoint_base,
            15,
            Duration::from_millis(1),
        )
        .unwrap()
    }

    #[test]
    fn rate_limiter_blocks_when_rpm_exceeded() {
        let limiter = GeminiRateLimiter::new(1);
        let now = Instant::now();
        assert!(limiter.try_acquire_at(now).is_ok());
        assert!(
            limiter
                .try_acquire_at(now + Duration::from_secs(1))
                .is_err()
        );
        assert!(
            limiter
                .try_acquire_at(now + Duration::from_secs(61))
                .is_ok()
        );
    }

    #[test]
    fn provider_refuses_when_api_key_missing() {
        let err =
            GeminiVlmProvider::new("", "model", DEFAULT_GEMINI_ENDPOINT_BASE, 15).unwrap_err();
        assert!(
            matches!(err, DocsError::VlmAuthentication { provider, .. } if provider == "gemini")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn health_check_succeeds_when_model_exists() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": [{"name": "models/gemini-3-flash-preview"}]
            })))
            .mount(&server)
            .await;
        let endpoint_base = server.uri();
        tokio::task::spawn_blocking(move || provider(endpoint_base).health_check())
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_sends_inlinedata_with_correct_mime() {
        let server = MockServer::start().await;
        let png = &[0x89, b'P', b'N', b'G', b'x'];
        Mock::given(method("POST"))
            .and(path("/models/gemini-3-flash-preview:generateContent"))
            .and(query_param("key", "test-key"))
            .and(body_json(json!({
                "contents": [{
                    "parts": [
                        {"text": IMAGE_DESCRIPTION_PROMPT},
                        {"inlineData": {"mimeType": "image/png", "data": STANDARD.encode(png)}}
                    ]
                }]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{"content": {"parts": [{"text": "upward line chart"}]}}]
            })))
            .mount(&server)
            .await;
        let endpoint_base = server.uri();
        let image = png.to_vec();
        let text =
            tokio::task::spawn_blocking(move || provider(endpoint_base).describe_image(&image))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(text, "upward line chart");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn describe_image_retries_on_429() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-3-flash-preview:generateContent"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-3-flash-preview:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{"content": {"parts": [{"text": "retried ok"}]}}]
            })))
            .mount(&server)
            .await;
        let endpoint_base = server.uri();
        let text = tokio::task::spawn_blocking(move || {
            provider(endpoint_base).describe_image(&[0x89, b'P', b'N', b'G'])
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(text, "retried ok");
    }
}
