use serde_json::{Map, Value};

const REDACTED: &str = "[redacted]";
const SENSITIVE_KEY_FRAGMENTS: &[&str] = &[
    "authorization",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "id_token",
    "token",
    "cookie",
    "password",
    "secret",
    "credential",
    "private_key",
    "ssh_agent",
];

pub fn redact_provider_metadata(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(redact_object(object)),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(redact_provider_metadata).collect())
        }
        other => other,
    }
}

fn redact_object(object: Map<String, Value>) -> Map<String, Value> {
    object
        .into_iter()
        .map(|(key, value)| {
            if is_sensitive_key(&key) {
                (key, Value::String(REDACTED.to_string()))
            } else {
                (key, redact_provider_metadata(value))
            }
        })
        .collect()
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .flat_map(char::to_lowercase)
        .collect::<String>();

    SENSITIVE_KEY_FRAGMENTS
        .iter()
        .any(|fragment| normalized.contains(fragment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_sensitive_keys_recursively() {
        let redacted = redact_provider_metadata(json!({
            "Authorization": "Bearer abc",
            "nested": {
                "refreshToken": "secret",
                "base_url": "https://api.example.com"
            },
            "headers": [
                {"x-api-key": "secret"},
                {"accept": "application/json"}
            ]
        }));

        assert_eq!(redacted["Authorization"], "[redacted]");
        assert_eq!(redacted["nested"]["refreshToken"], "[redacted]");
        assert_eq!(redacted["nested"]["base_url"], "https://api.example.com");
        assert_eq!(redacted["headers"][0]["x-api-key"], "[redacted]");
        assert_eq!(redacted["headers"][1]["accept"], "application/json");
    }

    #[test]
    fn leaves_non_sensitive_scalars_unchanged() {
        assert_eq!(redact_provider_metadata(json!("plain")), json!("plain"));
        assert_eq!(redact_provider_metadata(json!(42)), json!(42));
        assert_eq!(redact_provider_metadata(json!(true)), json!(true));
    }
}
