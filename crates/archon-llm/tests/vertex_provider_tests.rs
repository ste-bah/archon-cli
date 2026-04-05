/// Tests for Google Vertex AI provider adapter (TASK-CLI-404).
/// Written BEFORE implementation (Gate 01).
use archon_llm::provider::{LlmProvider, ProviderFeature};
use archon_llm::providers::VertexProvider;

// ---------------------------------------------------------------------------
// Test 1: VertexProvider implements LlmProvider (object-safe)
// ---------------------------------------------------------------------------

fn check_object_safe(_: Box<dyn LlmProvider>) {}

#[test]
fn vertex_provider_is_object_safe() {
    let provider = VertexProvider::new(
        "my-project".to_string(),
        "us-central1".to_string(),
        "claude-sonnet-4-20250514@20250514".to_string(),
        "anthropic".to_string(),
        None,
    );
    check_object_safe(Box::new(provider));
}

// ---------------------------------------------------------------------------
// Test 2a: Endpoint URL for Claude (publisher=anthropic)
// ---------------------------------------------------------------------------

#[test]
fn vertex_endpoint_url_claude_format() {
    let provider = VertexProvider::new(
        "my-project".to_string(),
        "us-central1".to_string(),
        "claude-sonnet-4-20250514@20250514".to_string(),
        "anthropic".to_string(),
        None,
    );
    let url = provider.endpoint_url();
    assert!(url.contains("us-central1-aiplatform.googleapis.com"), "URL should contain region: {url}");
    assert!(url.contains("my-project"), "URL should contain project: {url}");
    assert!(url.contains("publishers/anthropic"), "URL should contain publisher: {url}");
    assert!(url.contains("claude-sonnet-4-20250514"), "URL should contain model: {url}");
    assert!(url.ends_with(":streamGenerateContent"), "URL should end with :streamGenerateContent: {url}");
}

// ---------------------------------------------------------------------------
// Test 2b: Endpoint URL for Gemini (publisher=google)
// ---------------------------------------------------------------------------

#[test]
fn vertex_endpoint_url_gemini_format() {
    let provider = VertexProvider::new(
        "my-project".to_string(),
        "us-central1".to_string(),
        "gemini-1.5-pro".to_string(),
        "google".to_string(),
        None,
    );
    let url = provider.endpoint_url();
    assert!(url.contains("publishers/google"), "URL should contain google publisher: {url}");
    assert!(url.contains("gemini-1.5-pro"), "URL should contain model: {url}");
}

// ---------------------------------------------------------------------------
// Test 3: Service account JSON key parsed correctly
// ---------------------------------------------------------------------------

#[test]
fn gcp_service_account_key_parsed() {
    use archon_llm::providers::gcp_auth::ServiceAccountKey;

    let json = serde_json::json!({
        "type": "service_account",
        "client_email": "my-sa@my-project.iam.gserviceaccount.com",
        "private_key": "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...\n-----END RSA PRIVATE KEY-----\n",
        "token_uri": "https://oauth2.googleapis.com/token"
    });

    let key: ServiceAccountKey = serde_json::from_value(json).expect("should parse service account key");
    assert_eq!(key.client_email, "my-sa@my-project.iam.gserviceaccount.com");
    assert_eq!(key.token_uri, "https://oauth2.googleapis.com/token");
    assert!(key.private_key.contains("BEGIN RSA PRIVATE KEY"));
}

// ---------------------------------------------------------------------------
// Test 4: JWT has correct claims structure
// ---------------------------------------------------------------------------

#[test]
fn gcp_jwt_has_correct_claims_structure() {
    use archon_llm::providers::gcp_auth::build_jwt_claims;

    let claims = build_jwt_claims(
        "my-sa@my-project.iam.gserviceaccount.com",
        "https://oauth2.googleapis.com/token",
    );

    assert_eq!(claims["iss"], "my-sa@my-project.iam.gserviceaccount.com");
    assert_eq!(claims["aud"], "https://oauth2.googleapis.com/token");
    assert_eq!(claims["scope"], "https://www.googleapis.com/auth/cloud-platform");
    assert!(claims["iat"].is_number(), "iat should be a number");
    assert!(claims["exp"].is_number(), "exp should be a number");

    let iat = claims["iat"].as_i64().unwrap();
    let exp = claims["exp"].as_i64().unwrap();
    assert!(exp > iat, "exp should be after iat");
    assert_eq!(exp - iat, 3600, "token should expire in 3600 seconds");
}

// ---------------------------------------------------------------------------
// Test 5: ADC file path is correct
// ---------------------------------------------------------------------------

#[test]
fn gcp_adc_path_correct() {
    use archon_llm::providers::gcp_auth::adc_file_path;

    let path = adc_file_path();
    // Path should end with application_default_credentials.json
    assert!(
        path.ends_with("application_default_credentials.json"),
        "ADC path should end with application_default_credentials.json, got: {path}"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Missing credentials returns Err
// ---------------------------------------------------------------------------

#[test]
fn vertex_missing_credentials_returns_err() {
    use archon_llm::providers::gcp_auth::load_credentials_from_path;

    // Non-existent path should return Err.
    let result = load_credentials_from_path("/nonexistent/path/to/credentials.json");
    assert!(result.is_err(), "should return Err for missing credentials file");
}

// ---------------------------------------------------------------------------
// Test 7a: Claude on Vertex supports Thinking
// ---------------------------------------------------------------------------

#[test]
fn vertex_claude_supports_thinking() {
    let provider = VertexProvider::new(
        "proj".to_string(),
        "us-central1".to_string(),
        "claude-sonnet-4-20250514@20250514".to_string(),
        "anthropic".to_string(),
        None,
    );
    assert!(provider.supports_feature(ProviderFeature::Thinking));
    assert!(provider.supports_feature(ProviderFeature::PromptCaching));
    assert!(provider.supports_feature(ProviderFeature::Vision));
}

// ---------------------------------------------------------------------------
// Test 7b: Gemini on Vertex does NOT support Thinking
// ---------------------------------------------------------------------------

#[test]
fn vertex_gemini_no_thinking() {
    let provider = VertexProvider::new(
        "proj".to_string(),
        "us-central1".to_string(),
        "gemini-1.5-pro".to_string(),
        "google".to_string(),
        None,
    );
    assert!(!provider.supports_feature(ProviderFeature::Thinking));
    assert!(!provider.supports_feature(ProviderFeature::PromptCaching));
    // Gemini still supports Vision and ToolUse
    assert!(provider.supports_feature(ProviderFeature::Vision));
    assert!(provider.supports_feature(ProviderFeature::ToolUse));
}
