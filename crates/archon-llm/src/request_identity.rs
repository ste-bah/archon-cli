//! Hash only nonsecret request material. Credentials belong to transport headers.
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) fn digest(endpoint: &str, mut body: Value) -> Option<String> {
    let url = reqwest::Url::parse(endpoint).ok()?;
    // Embedded URL credentials and arbitrary query values may be secrets.
    // Do not cache these routes rather than hashing or discarding their values.
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    if let Some(metadata) = body.get_mut("metadata").and_then(Value::as_object_mut) {
        // Anthropic's user_id is device/account/session attribution, not a prompt.
        metadata.remove("user_id");
        if metadata.is_empty() {
            body.as_object_mut()?.remove("metadata");
        }
    }
    let envelope = serde_json::json!({"endpoint": url.as_str(), "body": body});
    Some(hex::encode(Sha256::digest(
        serde_json::to_vec(&envelope).ok()?,
    )))
}

#[cfg(test)]
#[path = "request_identity_tests.rs"]
mod tests;
