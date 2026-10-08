use super::*;
use serde_json::json;

/// A PEM private key block, built at run time so no literal key sits in the
/// source.
pub(crate) fn pem_block() -> String {
    let kind = "PRIVATE KEY";
    format!(
        "-----BEGIN {kind}-----\n{}\n-----END {kind}-----",
        "MIIEvQIBADANBgkqhkiG9w0BAQEFAASC".repeat(40)
    )
}

#[test]
fn a_write_keeps_its_path_and_never_its_content() {
    let pem = pem_block();
    let input = json!({"file_path": "src/token_store.rs", "content": pem});
    let safe = safe_input("Write", &input, 384);
    assert_eq!(safe.input, json!({"file_path": "src/token_store.rs"}));
    assert_eq!(safe.dropped_keys, 1);
    assert!(!safe.cut);
}

#[test]
fn an_edit_drops_old_and_new_text() {
    let input = json!({"file_path": "a.rs", "old_string": "PASSWORD=hunter2", "new_string": "x"});
    let safe = safe_input("Edit", &input, 384);
    assert_eq!(safe.input, json!({"file_path": "a.rs"}));
    assert_eq!(safe.dropped_keys, 2);
}

#[test]
fn a_url_keeps_scheme_host_and_path_only() {
    let input =
        json!({"url": "https://user:pw@example.invalid/v1/x?key=AIzaSECRET#frag", "prompt": "p"});
    let safe = safe_input("WebFetch", &input, 384);
    assert_eq!(safe.input, json!({"url": "https://example.invalid/v1/x"}));
    assert_eq!(
        strip_url("postgres://app:s3cret@db:5432/main"),
        "postgres://db:5432/main"
    );
}

#[test]
fn bash_keeps_its_command_with_credential_values_replaced() {
    let command = "export DB_PASSWORD=hunter2 FOO_TOKEN='abc def' MODE=fast; \
                   curl -H 'Authorization: Bearer abcdefgh' https://u:p@h.invalid/x?api_key=zzz";
    let safe = safe_input("Bash", &json!({"command": command, "timeout": 5}), 4096);
    let kept = safe.input["command"].as_str().unwrap();
    for secret in ["hunter2", "abc def", "abcdefgh", "u:p@", "zzz"] {
        assert!(!kept.contains(secret), "{secret} in {kept}");
    }
    assert!(
        kept.contains("MODE=fast") && kept.contains("DB_PASSWORD="),
        "{kept}"
    );
    assert_eq!(safe.dropped_keys, 1, "timeout is not on the allow-list");
}

#[test]
fn a_command_key_of_another_tool_is_dropped() {
    let safe = safe_input("mcp__srv__run", &json!({"command": "PASSWORD=x"}), 384);
    assert_eq!(safe.input, json!({}));
    assert_eq!(safe.dropped_keys, 1);
}

#[test]
fn a_secret_is_redacted_before_a_value_is_cut() {
    let command = format!("echo '{}'", pem_block());
    let safe = safe_input("Bash", &json!({"command": command}), 64);
    let kept = safe.input["command"].as_str().unwrap();
    assert!(!kept.contains("MIIEvQ"), "{kept}");
    assert!(!safe.cut, "the redacted command fits");
    let long = format!("grep {}", "x".repeat(500));
    let safe = safe_input("Bash", &json!({"command": long}), 64);
    assert!(safe.cut && safe.input["command"].as_str().unwrap().len() <= 64);
}

#[test]
fn search_text_loses_assignment_values_and_paths_keep_words() {
    let safe = safe_input(
        "Grep",
        &json!({"pattern": "PASSWORD=hunter2", "path": "config/secret_rules.md", "glob": "*.rs"}),
        384,
    );
    assert_eq!(safe.input["pattern"], "PASSWORD=[REDACTED]");
    assert_eq!(safe.input["path"], "config/secret_rules.md");
    assert_eq!(safe.input["glob"], "*.rs");
}

#[test]
fn numbers_are_kept_only_for_offset_and_limit() {
    let safe = safe_input(
        "Read",
        &json!({"file_path": "a", "offset": 3, "limit": "9"}),
        384,
    );
    assert_eq!(safe.input, json!({"file_path": "a", "offset": 3}));
    assert_eq!(safe.dropped_keys, 1);
    let safe = safe_input("Odd", &json!(["a", "b"]), 384);
    assert_eq!((safe.input, safe.dropped_keys), (Value::Null, 1));
}
