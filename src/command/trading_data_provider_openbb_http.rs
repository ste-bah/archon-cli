use serde_json::{Map, Value, json};

use super::metadata::credential_env_keys_for;
use super::request::OpenBbNativeRequest;

pub(super) struct OpenBbHttpResponse {
    pub(super) body: Vec<u8>,
    pub(super) redacted_headers: Value,
}

pub(super) fn fetch_openbb_response(
    base_url: &str,
    request: &OpenBbNativeRequest,
) -> Result<OpenBbHttpResponse, String> {
    require_credentials(request)?;
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        request.endpoint.trim_start_matches('/')
    );
    fetch_response_blocking(url, request)
}

fn fetch_response_blocking(
    url: String,
    request: &OpenBbNativeRequest,
) -> Result<OpenBbHttpResponse, String> {
    run_reqwest_blocking(|| request_openbb(url, request))
}

fn request_openbb(
    url: String,
    request: &OpenBbNativeRequest,
) -> Result<OpenBbHttpResponse, String> {
    let response = reqwest::blocking::Client::new()
        .get(url)
        .query(&request.params)
        .send()
        .map_err(|err| format!("OpenBB API request failed: {err}"))?;
    let status = response.status();
    let redacted_headers = redacted_response_headers(status.as_u16(), response.headers());
    if status.as_u16() == 204 {
        return Err("OpenBB API returned no content for the requested native dataset".into());
    }
    if !status.is_success() {
        return Err(format!(
            "OpenBB API returned HTTP {}; env keys checked: {}; headers captured redacted",
            status,
            credential_env_keys(request).join(",")
        ));
    }
    let body = response
        .bytes()
        .map_err(|err| format!("OpenBB API response body was unreadable: {err}"))?
        .to_vec();
    Ok(OpenBbHttpResponse {
        body,
        redacted_headers,
    })
}

fn run_reqwest_blocking<F>(operation: F) -> Result<OpenBbHttpResponse, String>
where
    F: FnOnce() -> Result<OpenBbHttpResponse, String>,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        // `reqwest::blocking` owns an internal Tokio runtime. Running and
        // dropping it inside a Tokio worker panics, so keep this synchronous
        // command path inside Tokio's blocking section when invoked by the CLI.
        return tokio::task::block_in_place(operation);
    }
    operation()
}

fn require_credentials(request: &OpenBbNativeRequest) -> Result<(), String> {
    if request.openbb_provider.eq_ignore_ascii_case("yfinance") {
        return Ok(());
    }
    let missing = request
        .credential_state
        .iter()
        .filter_map(|(key, present)| (!present).then_some(key.as_str()))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "OpenBB credentials unavailable for {}; missing env keys: {}; env keys checked: {}",
            request.openbb_provider,
            missing.join(","),
            credential_env_keys(request).join(",")
        ));
    }
    Ok(())
}

fn credential_env_keys(request: &OpenBbNativeRequest) -> Vec<&'static str> {
    credential_env_keys_for(&request.openbb_provider)
}

fn redacted_response_headers(status: u16, headers: &reqwest::header::HeaderMap) -> Value {
    let mut values = Map::new();
    for (name, value) in headers {
        values.insert(
            name.as_str().to_ascii_lowercase(),
            Value::String(redacted_header_value(name.as_str(), value)),
        );
    }
    json!({
        "http_status": status,
        "headers": values,
        "redaction": "credential-bearing response headers are redacted"
    })
}

fn redacted_header_value(name: &str, value: &reqwest::header::HeaderValue) -> String {
    let lower = name.to_ascii_lowercase();
    if lower.contains("key") || lower.contains("token") || lower.contains("auth") {
        return "<redacted>".into();
    }
    value.to_str().unwrap_or("<non-utf8>").into()
}
