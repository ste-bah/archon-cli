//! EC2 instance metadata (IMDSv2) credential source for the Bedrock provider.
//!
//! This is what lets an attached EC2 instance profile work without writing any
//! static secret to the box. Credentials returned here are always temporary, so
//! they must be signed with `x-amz-security-token` — see
//! [`super::aws_auth::build_authorization_header`].

use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::provider::LlmError;
use crate::providers::aws_auth::AwsCredentials;

/// Link-local IMDS address. Overridable via `AWS_EC2_METADATA_SERVICE_ENDPOINT`.
const IMDS_DEFAULT_ENDPOINT: &str = "http://169.254.169.254";

/// IMDS lives on the local link, so anything slower than this means we are not
/// on EC2. Keeping it short stops non-EC2 machines from stalling on every call.
const IMDS_TIMEOUT: Duration = Duration::from_millis(1000);

/// Instance-profile credentials cached until shortly before they expire.
struct CachedCredentials {
    creds: AwsCredentials,
    expires_at: DateTime<Utc>,
}

static IMDS_CACHE: OnceLock<tokio::sync::Mutex<Option<CachedCredentials>>> = OnceLock::new();

fn imds_endpoint() -> String {
    std::env::var("AWS_EC2_METADATA_SERVICE_ENDPOINT")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| IMDS_DEFAULT_ENDPOINT.to_string())
}

fn imds_disabled() -> bool {
    std::env::var("AWS_EC2_METADATA_DISABLED")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Fetch instance-profile credentials from IMDSv2.
///
/// Returns `Ok(None)` when this host simply is not an EC2 instance with a
/// profile attached — that is the common case off-EC2 and must not be an error,
/// since it would mask the real "no credentials anywhere" message.
async fn fetch_imds_credentials() -> Result<Option<(AwsCredentials, DateTime<Utc>)>, LlmError> {
    if imds_disabled() {
        return Ok(None);
    }

    let base = imds_endpoint();
    let http = reqwest::Client::builder()
        .timeout(IMDS_TIMEOUT)
        .build()
        .map_err(|e| LlmError::Auth(format!("failed to build IMDS client: {e}")))?;

    // 1. IMDSv2 session token. Any failure here means "not on EC2" far more
    //    often than "EC2 is broken", so treat it as absence.
    let token = match http
        .put(format!("{base}/latest/api/token"))
        .header("x-aws-ec2-metadata-token-ttl-seconds", "21600")
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp.text().await.unwrap_or_default(),
        _ => return Ok(None),
    };
    if token.is_empty() {
        return Ok(None);
    }

    // 2. Role attached to the instance profile. Reachable IMDS with no role
    //    listed means no profile is attached.
    let role = match http
        .get(format!("{base}/latest/meta-data/iam/security-credentials/"))
        .header("x-aws-ec2-metadata-token", &token)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp.text().await.unwrap_or_default(),
        _ => return Ok(None),
    };
    let role = role.lines().next().unwrap_or("").trim().to_string();
    if role.is_empty() {
        return Ok(None);
    }

    // 3. The credentials themselves. Past this point IMDS has told us a profile
    //    exists, so failures are real errors worth surfacing.
    let resp = http
        .get(format!(
            "{base}/latest/meta-data/iam/security-credentials/{}",
            urlencoding::encode(&role)
        ))
        .header("x-aws-ec2-metadata-token", &token)
        .send()
        .await
        .map_err(|e| {
            LlmError::Auth(format!("IMDS credential fetch failed for role {role}: {e}"))
        })?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(LlmError::Auth(format!(
            "IMDS returned {status} for role {role}"
        )));
    }

    parse_credential_response(&body, &role).map(Some)
}

/// Parse the IMDS credential JSON document.
fn parse_credential_response(
    body: &str,
    role: &str,
) -> Result<(AwsCredentials, DateTime<Utc>), LlmError> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| LlmError::Auth(format!("malformed IMDS credential response: {e}")))?;
    let field = |name: &str| {
        json.get(name)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };

    let access_key_id = field("AccessKeyId");
    let secret_access_key = field("SecretAccessKey");
    let session_token = field("Token");
    if access_key_id.is_empty() || secret_access_key.is_empty() || session_token.is_empty() {
        return Err(LlmError::Auth(format!(
            "IMDS credential response for role {role} was missing required fields"
        )));
    }

    // Without a parsable expiry, treat the credentials as already stale so we
    // re-fetch next call rather than trusting an unknown lifetime.
    let expires_at = json
        .get("Expiration")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    Ok((
        AwsCredentials {
            access_key_id,
            secret_access_key,
            session_token: Some(session_token),
        },
        expires_at,
    ))
}

/// Instance-profile credentials, cached across calls.
///
/// IMDS costs three round trips, so hitting it on every inference request would
/// be wasteful. The lock is held across the fetch deliberately: concurrent
/// requests during a refresh queue behind one fetch instead of stampeding IMDS.
pub async fn resolve_credentials_from_imds() -> Result<Option<AwsCredentials>, LlmError> {
    let cache = IMDS_CACHE.get_or_init(|| tokio::sync::Mutex::new(None));
    let mut guard = cache.lock().await;

    // Refresh early so a long stream never signs with credentials that expire
    // mid-request.
    let refresh_threshold = Utc::now() + chrono::Duration::minutes(5);
    if let Some(cached) = guard.as_ref()
        && cached.expires_at > refresh_threshold
    {
        return Ok(Some(cached.creds.clone()));
    }

    match fetch_imds_credentials().await? {
        Some((creds, expires_at)) => {
            *guard = Some(CachedCredentials {
                creds: creds.clone(),
                expires_at,
            });
            Ok(Some(creds))
        }
        None => {
            *guard = None;
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "Code": "Success",
        "AccessKeyId": "ASIAEXAMPLE",
        "SecretAccessKey": "secret",
        "Token": "FwoGZXIvYXdzEXAMPLETOKEN",
        "Expiration": "2026-07-30T18:00:00Z"
    }"#;

    #[test]
    fn parses_instance_profile_credentials() {
        let (creds, expires_at) = parse_credential_response(SAMPLE, "role").unwrap();
        assert_eq!(creds.access_key_id, "ASIAEXAMPLE");
        assert_eq!(
            creds.session_token.as_deref(),
            Some("FwoGZXIvYXdzEXAMPLETOKEN")
        );
        assert_eq!(expires_at.to_rfc3339(), "2026-07-30T18:00:00+00:00");
    }

    #[test]
    fn rejects_response_without_a_session_token() {
        let body = r#"{"AccessKeyId":"ASIAEXAMPLE","SecretAccessKey":"secret"}"#;
        assert!(parse_credential_response(body, "role").is_err());
    }

    #[test]
    fn unparsable_expiry_is_treated_as_already_stale() {
        let body = r#"{"AccessKeyId":"a","SecretAccessKey":"b","Token":"c","Expiration":"soon"}"#;
        let (_, expires_at) = parse_credential_response(body, "role").unwrap();
        // Already past the 5-minute refresh threshold, so the next call re-fetches.
        assert!(expires_at < Utc::now() + chrono::Duration::minutes(5));
    }

    #[tokio::test]
    async fn disabled_env_short_circuits() {
        // SAFETY: single-threaded test; restored immediately below.
        unsafe { std::env::set_var("AWS_EC2_METADATA_DISABLED", "true") };
        let result = fetch_imds_credentials().await;
        unsafe { std::env::remove_var("AWS_EC2_METADATA_DISABLED") };
        assert!(matches!(result, Ok(None)));
    }
}
