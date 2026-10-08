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
    // User info goes first: a `#` or `?` in a password cannot cut early.
    assert_eq!(
        strip_url("postgres://app:Pa#ss@db/main"),
        "postgres://db/main"
    );
    assert_eq!(
        strip_url("https://u:p?x@h.invalid/a?q=1"),
        "https://h.invalid/a"
    );
    assert_eq!(
        strip_url("https://host.invalid/a#frag"),
        "https://host.invalid/a"
    );
}

/// Shell commands that carry a credential in forms no redaction rule can
/// know, each holding `hunter2`.
pub(crate) const LEAKY_COMMANDS: &[&str] = &[
    "mysql -uroot -phunter2 app",
    "psql --password hunter2 -h db",
    "sshpass -p hunter2 ssh deploy@host",
    "curl -u admin:hunter2 https://api.invalid/x",
    r#"curl -d "{\"password\":\"hunter2\"}" https://api.invalid/login"#,
    "curl -H 'Authorization: Basic aHVudGVyMg==' https://api.invalid/x",
    "DB_PASSWORD=hunter2 ./deploy",
    "echo-free hunter2:x",
];

#[test]
fn bash_keeps_only_its_program_word_count_and_digest() {
    let safe = safe_input(
        "Bash",
        &json!({"command": "cargo test -p x -- --nocapture", "timeout": 5}),
        384,
    );
    assert_eq!(safe.input["program"], "cargo test");
    assert_eq!(safe.input["arg_count"], 5);
    assert_eq!(safe.input["command_sha256"].as_str().unwrap().len(), 64);
    assert!(safe.input.get("command").is_none());
    assert_eq!(safe.dropped_keys, 1, "timeout is not on the allow-list");
    for command in LEAKY_COMMANDS {
        let kept = safe_input("Bash", &json!({"command": command}), 384).input;
        let text = kept.to_string();
        assert!(
            !text.contains("hunter2") && !text.contains("aHVudGVyMg"),
            "{text}"
        );
    }
    assert_eq!(command_program("DB_PASSWORD=hunter2 ./deploy"), "");
    assert_eq!(command_program("git commit -m x"), "git commit");
    assert_eq!(command_program("a b c d e f"), "a b c d");
}

#[test]
fn a_command_key_of_another_tool_is_dropped() {
    let safe = safe_input("mcp__srv__run", &json!({"command": "PASSWORD=x"}), 384);
    assert_eq!(safe.input, json!({}));
    assert_eq!(safe.dropped_keys, 1);
}

#[test]
fn a_secret_is_redacted_before_a_value_is_cut() {
    let pattern = format!("{} {}", "x".repeat(10), pem_block());
    let safe = safe_input("Grep", &json!({"pattern": pattern}), 64);
    let kept = safe.input["pattern"].as_str().unwrap();
    assert!(!kept.contains("MIIEvQ"), "{kept}");
    assert!(!safe.cut, "the redacted pattern fits");
    let long = "x".repeat(500);
    let safe = safe_input("Grep", &json!({"pattern": long}), 64);
    assert!(safe.cut && safe.input["pattern"].as_str().unwrap().len() <= 64);
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
