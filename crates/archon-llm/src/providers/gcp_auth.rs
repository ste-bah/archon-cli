/// GCP authentication utilities for the Vertex AI provider.
///
/// Supports service account key files and Application Default Credentials (ADC).
/// Uses RS256 JWT signing via the `jsonwebtoken` crate.
use serde::{Deserialize, Serialize};

use crate::provider::LlmError;

// ---------------------------------------------------------------------------
// Service account key
// ---------------------------------------------------------------------------

/// A Google Cloud service account key parsed from a JSON key file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceAccountKey {
    /// Service account email address.
    pub client_email: String,
    /// PEM-encoded RSA private key.
    pub private_key: String,
    /// Token endpoint URL.
    pub token_uri: String,
}

/// An ADC file can be either a service account or an authorized_user credential.
/// We try to parse it as a service account first; if that fails, it's user credentials
/// which are not supported for server-to-server auth without refresh_token exchange.
#[derive(Debug, Clone, Deserialize)]
struct AdcFile {
    #[serde(rename = "type")]
    credential_type: String,
    client_email: Option<String>,
    private_key: Option<String>,
    token_uri: Option<String>,
}

// ---------------------------------------------------------------------------
// JWT claims
// ---------------------------------------------------------------------------

/// Build the JWT claims payload for a GCP service account token request.
///
/// The claims include `iss`, `scope`, `aud`, `iat`, and `exp` per the
/// [Google OAuth2 service account docs](https://developers.google.com/identity/protocols/oauth2/service-account).
pub fn build_jwt_claims(client_email: &str, token_uri: &str) -> serde_json::Value {
    let now = chrono::Utc::now().timestamp();
    serde_json::json!({
        "iss": client_email,
        "scope": "https://www.googleapis.com/auth/cloud-platform",
        "aud": token_uri,
        "iat": now,
        "exp": now + 3600
    })
}

// ---------------------------------------------------------------------------
// Signing
// ---------------------------------------------------------------------------

/// Sign a JWT with RS256 using the service account private key.
///
/// Returns the signed JWT string suitable for sending to the token endpoint.
pub fn sign_jwt(key: &ServiceAccountKey) -> Result<String, LlmError> {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

    #[derive(Serialize)]
    struct Claims {
        iss: String,
        scope: String,
        aud: String,
        iat: i64,
        exp: i64,
    }

    let now = chrono::Utc::now().timestamp();
    let claims = Claims {
        iss: key.client_email.clone(),
        scope: "https://www.googleapis.com/auth/cloud-platform".to_string(),
        aud: key.token_uri.clone(),
        iat: now,
        exp: now + 3600,
    };

    let header = Header::new(Algorithm::RS256);
    let encoding_key = EncodingKey::from_rsa_pem(key.private_key.as_bytes())
        .map_err(|e| LlmError::Auth(format!("invalid service account private key: {e}")))?;

    encode(&header, &claims, &encoding_key)
        .map_err(|e| LlmError::Auth(format!("JWT signing failed: {e}")))
}

// ---------------------------------------------------------------------------
// Token exchange
// ---------------------------------------------------------------------------

/// GCP access token with expiry information.
#[derive(Debug, Clone)]
pub struct GcpAccessToken {
    pub access_token: String,
    pub expires_at: std::time::Instant,
}

/// Exchange a signed JWT for a GCP access token via the token endpoint.
pub async fn get_access_token(
    http: &reqwest::Client,
    key: &ServiceAccountKey,
) -> Result<GcpAccessToken, LlmError> {
    let jwt = sign_jwt(key)?;

    let params = [
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", &jwt),
    ];

    let resp = http
        .post(&key.token_uri)
        .form(&params)
        .send()
        .await
        .map_err(|e| LlmError::Http(format!("GCP token request failed: {e}")))?;

    let status = resp.status().as_u16();
    if status >= 400 {
        let body = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
        return Err(LlmError::Auth(format!(
            "GCP token endpoint returned {status}: {body}"
        )));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        expires_in: Option<u64>,
    }

    let token_resp: TokenResponse = resp
        .json()
        .await
        .map_err(|e| LlmError::Serialize(format!("failed to parse GCP token response: {e}")))?;

    let expires_secs = token_resp.expires_in.unwrap_or(3600);
    let expires_at =
        std::time::Instant::now() + std::time::Duration::from_secs(expires_secs.saturating_sub(60));

    Ok(GcpAccessToken {
        access_token: token_resp.access_token,
        expires_at,
    })
}

// ---------------------------------------------------------------------------
// Credential loading
// ---------------------------------------------------------------------------

/// Return the path to the ADC file as a string.
pub fn adc_file_path() -> String {
    let base = dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from(".config"));
    base.join("gcloud")
        .join("application_default_credentials.json")
        .to_string_lossy()
        .to_string()
}

/// Load service account credentials from a specific file path.
///
/// Returns `Err` if the file is missing or cannot be parsed as a service account key.
pub fn load_credentials_from_path(path: &str) -> Result<ServiceAccountKey, LlmError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        LlmError::Auth(format!("failed to read GCP credentials file '{path}': {e}"))
    })?;

    let key: ServiceAccountKey = serde_json::from_str(&content).map_err(|e| {
        LlmError::Auth(format!(
            "failed to parse GCP credentials file '{path}': {e}"
        ))
    })?;

    Ok(key)
}

/// Try to load credentials from Application Default Credentials.
///
/// Returns `Ok(None)` if the ADC file is absent or is not a service account type.
pub fn load_adc() -> Option<ServiceAccountKey> {
    let path = adc_file_path();

    let content = std::fs::read_to_string(&path).ok()?;
    let adc: AdcFile = serde_json::from_str(&content).ok()?;

    if adc.credential_type != "service_account" {
        // authorized_user type — not supported for direct signing.
        return None;
    }

    let client_email = adc.client_email?;
    let private_key = adc.private_key?;
    let token_uri = adc
        .token_uri
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string());

    Some(ServiceAccountKey {
        client_email,
        private_key,
        token_uri,
    })
}

/// Resolve GCP credentials from a file path override or ADC.
///
/// Returns `Err` if no credentials are found.
pub fn resolve_credentials(credentials_file: Option<&str>) -> Result<ServiceAccountKey, LlmError> {
    // 1. Try explicit credentials file.
    if let Some(path) = credentials_file {
        return load_credentials_from_path(path);
    }

    // 2. Try GOOGLE_APPLICATION_CREDENTIALS env var.
    if let Ok(path) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
        if !path.is_empty() {
            return load_credentials_from_path(&path);
        }
    }

    // 3. Try ADC file.
    if let Some(key) = load_adc() {
        return Ok(key);
    }

    Err(LlmError::Auth(
        "GCP credentials not found. Set GOOGLE_APPLICATION_CREDENTIALS or configure \
         application default credentials with `gcloud auth application-default login`"
            .to_string(),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_account_key_deserialized() {
        let json = r#"{
            "type": "service_account",
            "client_email": "sa@project.iam.gserviceaccount.com",
            "private_key": "-----BEGIN RSA PRIVATE KEY-----\nfake\n-----END RSA PRIVATE KEY-----\n",
            "token_uri": "https://oauth2.googleapis.com/token"
        }"#;
        let key: ServiceAccountKey = serde_json::from_str(json).unwrap();
        assert_eq!(key.client_email, "sa@project.iam.gserviceaccount.com");
        assert_eq!(key.token_uri, "https://oauth2.googleapis.com/token");
    }

    #[test]
    fn adc_path_ends_with_correct_filename() {
        let path = adc_file_path();
        assert!(path.ends_with("application_default_credentials.json"));
    }

    #[test]
    fn jwt_claims_have_correct_scope() {
        let claims = build_jwt_claims(
            "sa@project.iam.gserviceaccount.com",
            "https://oauth2.googleapis.com/token",
        );
        assert_eq!(
            claims["scope"],
            "https://www.googleapis.com/auth/cloud-platform"
        );
    }
}
