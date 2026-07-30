/// AWS SigV4 credential resolution and request signing for the Bedrock provider.
///
/// Resolves credentials from environment variables, `~/.aws/credentials`, and the
/// EC2 instance metadata service (see [`crate::providers::aws_imds`]), then signs
/// requests with hand-rolled SigV4 HMAC-SHA256 rather than the full AWS SDK.
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::provider::LlmError;
use crate::providers::aws_imds::resolve_credentials_from_imds;

// ---------------------------------------------------------------------------
// Credentials
// ---------------------------------------------------------------------------

/// Resolved AWS credentials.
#[derive(Debug, Clone)]
pub struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

/// Resolve credentials from environment variables only (no file fallback).
///
/// Returns `Ok(None)` if the env vars are not set, `Err` only on unexpected failure.
pub fn resolve_credentials_no_file() -> Result<Option<AwsCredentials>, LlmError> {
    let key = std::env::var("AWS_ACCESS_KEY_ID").ok();
    let secret = std::env::var("AWS_SECRET_ACCESS_KEY").ok();

    match (key, secret) {
        (Some(k), Some(s)) if !k.is_empty() && !s.is_empty() => {
            let token = std::env::var("AWS_SESSION_TOKEN").ok();
            Ok(Some(AwsCredentials {
                access_key_id: k,
                secret_access_key: s,
                session_token: token,
            }))
        }
        _ => Ok(None),
    }
}

/// Resolve AWS credentials: env vars, then `~/.aws/credentials`, then the EC2
/// instance metadata service.
///
/// The IMDS step is what makes an attached EC2 instance profile work without
/// writing any static secret to the box.
///
/// Returns `Err` with a descriptive message if no source is available.
pub async fn resolve_credentials() -> Result<AwsCredentials, LlmError> {
    // 1. Check env vars.
    if let Some(creds) = resolve_credentials_no_file()? {
        return Ok(creds);
    }

    // 2. Try ~/.aws/credentials.
    if let Some(creds) = load_credentials_file()? {
        return Ok(creds);
    }

    // 3. Try the instance profile (EC2 only; cheap no-op elsewhere).
    if let Some(creds) = resolve_credentials_from_imds().await? {
        return Ok(creds);
    }

    Err(LlmError::Auth(
        "AWS credentials not found. Set AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY, \
         configure ~/.aws/credentials, or attach an EC2 instance profile"
            .to_string(),
    ))
}

/// Parse the default AWS credentials file (`~/.aws/credentials`).
///
/// Returns the `[default]` profile if found.
fn load_credentials_file() -> Result<Option<AwsCredentials>, LlmError> {
    let creds_path = match dirs::home_dir() {
        Some(h) => h.join(".aws").join("credentials"),
        None => return Ok(None),
    };

    if !creds_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&creds_path)
        .map_err(|e| LlmError::Auth(format!("failed to read ~/.aws/credentials: {e}")))?;

    parse_ini_credentials(&content)
}

/// Parse INI-format credentials file, returning the `[default]` profile.
fn parse_ini_credentials(content: &str) -> Result<Option<AwsCredentials>, LlmError> {
    let mut in_default = false;
    let mut access_key = None::<String>;
    let mut secret_key = None::<String>;
    let mut session_token = None::<String>;

    for line in content.lines() {
        let line = line.trim();

        if line.starts_with('[') && line.ends_with(']') {
            // Section header: save previous default if found.
            if in_default {
                break;
            }
            in_default = line == "[default]";
            continue;
        }

        if !in_default {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().to_string();
            match key {
                "aws_access_key_id" => access_key = Some(value),
                "aws_secret_access_key" => secret_key = Some(value),
                "aws_session_token" => session_token = Some(value),
                _ => {}
            }
        }
    }

    match (access_key, secret_key) {
        (Some(k), Some(s)) if !k.is_empty() && !s.is_empty() => Ok(Some(AwsCredentials {
            access_key_id: k,
            secret_access_key: s,
            session_token,
        })),
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// SigV4 signing
// ---------------------------------------------------------------------------

/// Compute SHA-256 hash of bytes and return as lowercase hex string.
fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Compute HMAC-SHA256.
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Build a SigV4 `Authorization` header value for an HTTP request.
///
/// # Arguments
/// - `creds` — resolved AWS credentials
/// - `method` — HTTP method (e.g. `"POST"`)
/// - `host` — hostname without scheme (e.g. `"bedrock-runtime.us-east-1.amazonaws.com"`)
/// - `path` — URL path (e.g. `"/model/anthropic.claude-v2/converse-stream"`)
/// - `region` — AWS region
/// - `service` — AWS service name (e.g. `"bedrock"`)
/// - `body` — request body bytes
/// - `now` — timestamp for signing
#[allow(clippy::too_many_arguments)]
pub fn build_authorization_header(
    creds: &AwsCredentials,
    method: &str,
    host: &str,
    path: &str,
    region: &str,
    service: &str,
    body: &[u8],
    now: DateTime<Utc>,
) -> String {
    let date_str = now.format("%Y%m%d").to_string();
    let datetime_str = now.format("%Y%m%dT%H%M%SZ").to_string();

    // 1. Canonical request
    //
    // Temporary credentials (SSO, assume-role, EC2 instance profiles) carry a
    // session token that must be signed as the `x-amz-security-token` header —
    // AWS rejects the request otherwise. Canonical headers are sorted by name,
    // and `x-amz-security-token` sorts after `x-amz-date`.
    let payload_hash = sha256_hex(body);
    let mut canonical_headers =
        format!("content-type:application/json\nhost:{host}\nx-amz-date:{datetime_str}\n");
    let mut signed_headers = "content-type;host;x-amz-date".to_string();
    if let Some(ref token) = creds.session_token {
        canonical_headers.push_str(&format!("x-amz-security-token:{token}\n"));
        signed_headers.push_str(";x-amz-security-token");
    }
    let canonical_uri = path;
    let canonical_query_string = "";

    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query_string}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );

    // 2. String to sign
    let credential_scope = format!("{date_str}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{datetime_str}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );

    // 3. Signing key
    let k_date = hmac_sha256(
        format!("AWS4{}", creds.secret_access_key).as_bytes(),
        date_str.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");

    // 4. Signature
    let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    // 5. Authorization header. The session token travels as its own request
    //    header (signed above), not as an extra field here.
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
        creds.access_key_id
    )
}

/// Headers required on a signed Bedrock request.
pub struct SignedHeaders {
    /// `x-amz-date` — must match the timestamp the signature was computed over.
    pub x_amz_date: String,
    /// `authorization` — the SigV4 credential/signature header.
    pub authorization: String,
    /// `x-amz-security-token` — present only for temporary credentials.
    pub security_token: Option<String>,
}

/// Build the headers needed for a signed Bedrock request.
pub fn signed_headers(
    creds: &AwsCredentials,
    host: &str,
    path: &str,
    region: &str,
    body: &[u8],
) -> SignedHeaders {
    // One timestamp for both the signature and the header it is sent in; taking
    // `Utc::now()` twice can straddle a second boundary and invalidate the
    // signature.
    let now = Utc::now();
    SignedHeaders {
        x_amz_date: now.format("%Y%m%dT%H%M%SZ").to_string(),
        authorization: build_authorization_header(
            creds, "POST", host, path, region, "bedrock", body, now,
        ),
        security_token: creds.session_token.clone(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ini_credentials_default_profile() {
        let ini = "[default]\naws_access_key_id = AKIA123\naws_secret_access_key = SECRET\n";
        let creds = parse_ini_credentials(ini).unwrap().unwrap();
        assert_eq!(creds.access_key_id, "AKIA123");
        assert_eq!(creds.secret_access_key, "SECRET");
    }

    #[test]
    fn parse_ini_credentials_missing_returns_none() {
        let ini = "[other-profile]\naws_access_key_id = AKIA123\naws_secret_access_key = SECRET\n";
        let result = parse_ini_credentials(ini).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn authorization_header_has_correct_prefix() {
        let creds = AwsCredentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: None,
        };
        let now = chrono::Utc::now();
        let auth = build_authorization_header(
            &creds,
            "POST",
            "bedrock-runtime.us-east-1.amazonaws.com",
            "/model/test/converse-stream",
            "us-east-1",
            "bedrock",
            b"{}",
            now,
        );
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"));
    }

    fn temp_creds(session_token: Option<&str>) -> AwsCredentials {
        AwsCredentials {
            access_key_id: "ASIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            session_token: session_token.map(str::to_string),
        }
    }

    fn sign(creds: &AwsCredentials, now: DateTime<Utc>) -> String {
        build_authorization_header(
            creds,
            "POST",
            "bedrock-runtime.us-east-1.amazonaws.com",
            "/model/test/converse-stream",
            "us-east-1",
            "bedrock",
            b"{}",
            now,
        )
    }

    #[test]
    fn session_token_is_signed_not_appended() {
        let auth = sign(&temp_creds(Some("FwoGZXIvYXdzEXAMPLETOKEN")), Utc::now());

        // The token belongs in SignedHeaders, never as a trailing Authorization
        // field — the old form was silently rejected by AWS.
        assert!(
            auth.contains("SignedHeaders=content-type;host;x-amz-date;x-amz-security-token"),
            "session token missing from SignedHeaders: {auth}"
        );
        assert!(
            !auth.contains("X-Amz-Security-Token="),
            "session token leaked into the Authorization header: {auth}"
        );
    }

    #[test]
    fn static_credentials_omit_security_token_header() {
        let auth = sign(&temp_creds(None), Utc::now());
        assert!(auth.contains("SignedHeaders=content-type;host;x-amz-date,"));
        assert!(!auth.contains("x-amz-security-token"));
    }

    #[test]
    fn session_token_changes_the_signature() {
        let now = Utc::now();
        let without = sign(&temp_creds(None), now);
        let with = sign(&temp_creds(Some("FwoGZXIvYXdzEXAMPLETOKEN")), now);
        assert_ne!(
            without, with,
            "token must be covered by the signature, not ignored"
        );
    }

    #[test]
    fn signed_headers_surfaces_the_session_token() {
        let creds = temp_creds(Some("FwoGZXIvYXdzEXAMPLETOKEN"));
        let headers = signed_headers(
            &creds,
            "bedrock-runtime.us-east-1.amazonaws.com",
            "/model/test/converse-stream",
            "us-east-1",
            b"{}",
        );
        assert_eq!(
            headers.security_token.as_deref(),
            Some("FwoGZXIvYXdzEXAMPLETOKEN")
        );

        // x-amz-date must be the exact timestamp inside the credential scope,
        // otherwise the signature does not verify.
        let date = &headers.x_amz_date[..8];
        assert!(
            headers
                .authorization
                .contains(&format!("/{date}/us-east-1/bedrock/aws4_request")),
            "x-amz-date disagrees with the signed credential scope: {headers:?}",
            headers = headers.authorization
        );
    }

    #[test]
    fn signed_headers_omits_token_for_static_credentials() {
        let headers = signed_headers(
            &temp_creds(None),
            "bedrock-runtime.us-east-1.amazonaws.com",
            "/model/test/converse-stream",
            "us-east-1",
            b"{}",
        );
        assert!(headers.security_token.is_none());
    }
}
