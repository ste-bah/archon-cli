use super::*;

#[test]
fn round2_literal_redaction_handles_escaped_values() {
    let _registry = super::scoped_registry_for_tests();
    let secrets = SecretValues::new(["quoted\"obs\\value"]);
    for text in ["quoted\"obs\\value", "quoted\\\"obs\\\\value"] {
        assert_eq!(secrets.text(text), REDACTED_VALUE);
    }
}

#[test]
fn round2_literal_redaction_handles_url_encoded_values() {
    let _registry = super::scoped_registry_for_tests();
    let secrets = SecretValues::new(["obs/url+value with space"]);
    for text in [
        "obs%2Furl%2Bvalue%20with%20space",
        "obs%2furl%2bvalue%20with%20space",
        "obs%2Furl%2Bvalue+with+space",
    ] {
        assert_eq!(secrets.text(text), REDACTED_VALUE);
    }
}

#[test]
fn round2_registry_can_be_scoped_without_cross_test_pollution() {
    let _registry = super::scoped_registry_for_tests();
    let value = "registry-round2-isolation-canary";
    SecretValues::new([value]).register();
    assert_eq!(redact_registered(value), REDACTED_VALUE);
    assert_eq!(
        std::thread::spawn(move || redact_registered(value))
            .join()
            .expect("thread"),
        value
    );
}

#[test]
fn python_ascii_json_secret_spellings_are_redacted() {
    for (secret, escaped) in [
        ("credential-é-canary", r"credential-\u00e9-canary"),
        ("credential-\u{007f}-canary", r"credential-\u007f-canary"),
        ("credential-中-canary", r"credential-\u4e2d-canary"),
        ("credential-😀-canary", r"credential-\ud83d\ude00-canary"),
    ] {
        assert_eq!(SecretValues::new([secret]).text(escaped), REDACTED_VALUE);
    }
}
