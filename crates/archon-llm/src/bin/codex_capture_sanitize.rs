use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    if args.source != "openclaw" {
        bail!("capture source must be openclaw; capturing real Codex CLI traffic is prohibited");
    }

    let input = fs::read_to_string(&args.input)
        .with_context(|| format!("failed to read {}", args.input.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&input).context("input must be HAR/flow JSON")?;
    let sanitized = sanitize_value(value);

    fs::create_dir_all(&args.output)
        .with_context(|| format!("failed to create {}", args.output.display()))?;
    let output_path = args.output.join("sanitized_capture.json");
    let envelope = serde_json::json!({
        "source": args.source,
        "source_version": args.source_version,
        "sanitized": sanitized,
    });
    fs::write(&output_path, serde_json::to_string_pretty(&envelope)?)
        .with_context(|| format!("failed to write {}", output_path.display()))?;
    println!("wrote {}", output_path.display());
    Ok(())
}

#[derive(Debug)]
struct Args {
    input: PathBuf,
    output: PathBuf,
    source: String,
    source_version: String,
}

fn parse_args<I>(args: I) -> Result<Args>
where
    I: IntoIterator<Item = String>,
{
    let mut values = BTreeMap::new();
    let mut iter = args.into_iter();
    while let Some(flag) = iter.next() {
        if !flag.starts_with("--") {
            bail!("unexpected positional argument `{flag}`");
        }
        let value = iter
            .next()
            .with_context(|| format!("missing value for {flag}"))?;
        values.insert(flag, value);
    }

    Ok(Args {
        input: required_path(&mut values, "--input")?,
        output: required_path(&mut values, "--output")?,
        source: required_string(&mut values, "--source")?,
        source_version: required_string(&mut values, "--source-version")?,
    })
}

fn required_path(values: &mut BTreeMap<String, String>, key: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(required_string(values, key)?))
}

fn required_string(values: &mut BTreeMap<String, String>, key: &str) -> Result<String> {
    values
        .remove(key)
        .with_context(|| format!("missing required {key}"))
}

fn sanitize_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let sanitized = map
                .into_iter()
                .map(|(key, value)| {
                    let replacement = replacement_for_key(&key);
                    let value = replacement
                        .map(serde_json::Value::String)
                        .unwrap_or_else(|| sanitize_value(value));
                    (key, value)
                })
                .collect();
            serde_json::Value::Object(sanitized)
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(sanitize_value).collect())
        }
        serde_json::Value::String(value) => serde_json::Value::String(sanitize_string(&value)),
        other => other,
    }
}

fn replacement_for_key(key: &str) -> Option<String> {
    match key.to_ascii_lowercase().as_str() {
        "authorization" | "access_token" | "accesstoken" => Some("Bearer {{ACCESS_TOKEN}}".into()),
        "refresh_token" | "refreshtoken" => Some("{{REFRESH_TOKEN}}".into()),
        "chatgpt-account-id" | "accountid" | "account_id" => Some("{{ACCOUNT_ID}}".into()),
        "session_id" | "x-client-request-id" | "request_id" => Some("{{SESSION_ID}}".into()),
        "code" | "code_verifier" => Some("{{OAUTH_CODE}}".into()),
        _ => None,
    }
}

fn sanitize_string(value: &str) -> String {
    if value.starts_with("Bearer ") {
        return "Bearer {{ACCESS_TOKEN}}".into();
    }
    if value.contains("sk-") || value.contains("eyJ") {
        return "{{REDACTED_SECRET}}".into();
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_openclaw_source() {
        let result = parse_args([
            "--input".into(),
            "in.json".into(),
            "--output".into(),
            "out".into(),
            "--source".into(),
            "codex-cli".into(),
            "--source-version".into(),
            "1".into(),
        ])
        .and_then(|args| {
            if args.source != "openclaw" {
                bail!("capture source must be openclaw")
            }
            Ok(args)
        });

        assert!(matches!(result, Err(err) if err.to_string().contains("openclaw")));
    }

    #[test]
    fn sanitizes_secret_fields_recursively() {
        let value = serde_json::json!({
            "headers": {
                "authorization": "Bearer live-token",
                "chatgpt-account-id": "acct_real"
            },
            "body": {
                "refresh_token": "refresh-real",
                "nested": [{"session_id": "abc"}]
            }
        });

        let sanitized = sanitize_value(value);
        assert_eq!(
            sanitized["headers"]["authorization"],
            "Bearer {{ACCESS_TOKEN}}"
        );
        assert_eq!(sanitized["headers"]["chatgpt-account-id"], "{{ACCOUNT_ID}}");
        assert_eq!(sanitized["body"]["refresh_token"], "{{REFRESH_TOKEN}}");
        assert_eq!(
            sanitized["body"]["nested"][0]["session_id"],
            "{{SESSION_ID}}"
        );
    }
}
