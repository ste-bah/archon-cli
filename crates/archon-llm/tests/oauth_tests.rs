use archon_llm::oauth::{
    build_auth_url, generate_code_challenge, generate_code_verifier, generate_state,
    start_callback_server,
};

#[test]
fn code_verifier_is_43_chars_base64url() {
    let verifier = generate_code_verifier();
    // 32 bytes -> 43 base64url chars (no padding)
    assert_eq!(verifier.len(), 43, "verifier length should be 43, got {}", verifier.len());
    // No padding characters
    assert!(!verifier.contains('='), "verifier should not contain padding");
    // Only base64url characters
    assert!(
        verifier.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "verifier should only contain base64url chars"
    );
}

#[test]
fn code_challenge_is_sha256_of_verifier() {
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);

    // SHA256 hash -> 32 bytes -> 43 base64url chars
    assert_eq!(challenge.len(), 43, "challenge length should be 43");
    assert!(!challenge.contains('='));

    // Same verifier produces same challenge
    let challenge2 = generate_code_challenge(&verifier);
    assert_eq!(challenge, challenge2, "deterministic challenge");

    // Different verifier produces different challenge
    let verifier2 = generate_code_verifier();
    let challenge3 = generate_code_challenge(&verifier2);
    assert_ne!(challenge, challenge3, "different verifiers should differ");
}

#[test]
fn state_is_random_base64url() {
    let state = generate_state();
    assert_eq!(state.len(), 43);
    assert!(!state.contains('='));

    // Two calls produce different states
    let state2 = generate_state();
    assert_ne!(state, state2, "states should be random");
}

#[test]
fn auth_url_contains_all_parameters() {
    let challenge = "test-challenge-value";
    let state = "test-state-value";
    let port = 8765;

    let url = build_auth_url(challenge, state, port);

    assert!(url.starts_with("https://claude.com/cai/oauth/authorize?"), "wrong base URL");
    assert!(url.contains("response_type=code"), "missing response_type");
    assert!(url.contains("client_id=9d1c250a"), "missing client_id");
    assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8765%2Fcallback")
        || url.contains("redirect_uri=http://127.0.0.1:8765/callback"),
        "missing or wrong redirect_uri in: {url}");
    assert!(url.contains("state=test-state-value"), "missing state");
    assert!(url.contains("code_challenge=test-challenge-value"), "missing challenge");
    assert!(url.contains("code_challenge_method=S256"), "missing S256 method");
    assert!(url.contains("scope="), "missing scope");
    assert!(url.contains("user%3Aprofile") || url.contains("user:profile"), "missing profile scope");
}

#[test]
fn callback_server_starts_on_random_port() {
    let state = "test-state-123";
    let (port, _rx) = start_callback_server(state).expect("server should start");
    assert!(port > 0, "port should be assigned");
    // Server is running in a background thread -- rx will receive when callback arrives or times out
}

#[test]
fn callback_server_state_mismatch_rejected() {
    let (port, rx) = start_callback_server("correct-state").expect("server start");

    // Simulate a callback with wrong state
    std::thread::spawn(move || {
        let url = format!("http://127.0.0.1:{port}/callback?code=test-code&state=wrong-state");
        let _ = reqwest::blocking::get(&url);
    });

    let result = rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("should receive response");

    assert!(result.is_err(), "mismatched state should be rejected");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("state") || err_msg.contains("mismatch"), "error should mention state: {err_msg}");
}

#[test]
fn callback_server_error_param_handled() {
    let (port, rx) = start_callback_server("my-state").expect("server start");

    std::thread::spawn(move || {
        let url = format!(
            "http://127.0.0.1:{port}/callback?error=access_denied&error_description=User+denied&state=my-state"
        );
        let _ = reqwest::blocking::get(&url);
    });

    let result = rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("should receive response");

    assert!(result.is_err(), "error param should produce error");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("access_denied"), "error should contain error code: {err_msg}");
}

#[test]
fn callback_server_extracts_code() {
    let (port, rx) = start_callback_server("valid-state").expect("server start");

    std::thread::spawn(move || {
        let url = format!(
            "http://127.0.0.1:{port}/callback?code=auth-code-xyz&state=valid-state"
        );
        let _ = reqwest::blocking::get(&url);
    });

    let result = rx.recv_timeout(std::time::Duration::from_secs(5))
        .expect("should receive response");

    let code = result.expect("should succeed with valid state");
    assert_eq!(code, "auth-code-xyz");
}
