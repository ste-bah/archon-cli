use archon_llm::auth::AuthError;
use archon_llm::oauth_codex::parse_callback_url;

#[test]
fn state_mismatch_returns_specific_error() {
    let err = parse_callback_url("/auth/callback?code=abc&state=wrong", "expected")
        .expect_err("state mismatch");
    assert!(matches!(err, AuthError::StateMismatch));
}

#[test]
fn matching_state_returns_code() {
    let code = parse_callback_url("/auth/callback?code=abc&state=expected", "expected")
        .expect("matching state");
    assert_eq!(code, "abc");
}
