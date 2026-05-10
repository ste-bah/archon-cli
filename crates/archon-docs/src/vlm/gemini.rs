use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::json;

use crate::errors::DocsError;
use crate::vlm::mime::detect_mime;
use crate::vlm::retry::{RateLimitRetry, retry_rate_limited};
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
        let endpoint_base = normalize_endpoint_base(endpoint_base.into())?;
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
            endpoint_base,
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
        let url = self.endpoint_url("models")?;
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
        let url = self.endpoint_url(&format!("models/{}:generateContent", self.model))?;
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

    fn endpoint_url(&self, path: &str) -> Result<reqwest::Url, DocsError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/{}",
            self.endpoint_base,
            path.trim_start_matches('/')
        ))
        .map_err(|e| gemini_provider_error(format!("invalid Gemini endpoint URL: {e}")))?;
        url.query_pairs_mut().append_pair("key", &self.api_key);
        Ok(url)
    }
}

impl VlmDescriptionProvider for GeminiVlmProvider {
    fn describe_image(&self, image_bytes: &[u8]) -> Result<String, DocsError> {
        retry_rate_limited(RateLimitRetry::vlm_default(self.retry_base_delay), || {
            self.generate_once(image_bytes)
        })
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

fn normalize_endpoint_base(endpoint_base: String) -> Result<String, DocsError> {
    let endpoint_base = endpoint_base.trim().trim_end_matches('/').to_string();
    let url = reqwest::Url::parse(&endpoint_base)
        .map_err(|e| gemini_provider_error(format!("invalid Gemini endpoint_base: {e}")))?;
    if endpoint_allows_sensitive_data(&url) {
        Ok(endpoint_base)
    } else {
        Err(gemini_provider_error(
            "Gemini endpoint_base must use HTTPS unless it targets loopback/local test host",
        ))
    }
}

fn endpoint_allows_sensitive_data(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        || (url.scheme() == "http" && url.host_str().map(is_loopback_host).unwrap_or(false))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|addr| addr.is_loopback())
            .unwrap_or(false)
}

fn gemini_provider_error(message: impl Into<String>) -> DocsError {
    DocsError::VlmProvider {
        provider: "gemini".into(),
        message: message.into(),
        status_code: None,
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
#[path = "gemini_tests.rs"]
mod tests;
