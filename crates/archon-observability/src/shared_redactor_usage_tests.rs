use super::{is_credential_field, redact_json, redact_text};
use serde_json::json;

#[test]
fn exact_usage_count_fields_are_exempt_only_for_numeric_or_null_values() {
    for key in ["tokens_in", "tokens_out", "tokensin", "tokensout"] {
        for value in [json!(19), json!(null)] {
            assert!(!is_credential_field(key, &value));
            assert_eq!(
                redact_json(json!({key:value.clone()}), "[REDACTED]")[key],
                value
            );
        }
        let value = json!("measured-count");
        assert!(is_credential_field(key, &value));
        assert_eq!(
            redact_json(json!({key:value}), "[REDACTED]")[key],
            "[REDACTED]"
        );
    }
    for key in ["prefix_tokens_in", "tokens_out_suffix"] {
        let value = json!(19);
        assert!(is_credential_field(key, &value));
        assert_eq!(
            redact_json(json!({key:value}), "[REDACTED]")[key],
            "[REDACTED]"
        );
    }
    assert_eq!(redact_text("tokens_in=12", "[REDACTED]"), "tokens_in=12");
    assert_eq!(
        redact_text("tokens_in=abc", "[REDACTED]"),
        "tokens_in=[REDACTED]"
    );
    assert_eq!(
        redact_text(r#"{"tokens_in":"12"}"#, "[REDACTED]"),
        r#"{"tokens_in":"[REDACTED]"}"#
    );
}
