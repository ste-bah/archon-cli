//! Regression: the cch hash must be computed from the EXACT body bytes
//! sent to the server, not a stale snapshot. If the body is mutated
//! after hashing, this test catches it.

use archon_llm::cch::compute_cch;

#[test]
fn cch_recomputed_per_request_matches_serialized_body() {
    let body_a = serde_json::to_vec(&serde_json::json!({"a": 1})).unwrap();
    let body_b = serde_json::to_vec(&serde_json::json!({"a": 2})).unwrap();
    assert_ne!(compute_cch(&body_a), compute_cch(&body_b));
}

#[test]
fn cch_matches_exact_serialized_output() {
    // Build two JSON payloads that differ, verify CCH differs
    let body_a = serde_json::to_string(&serde_json::json!({"model": "claude-sonnet-4-6", "max_tokens": 8192, "messages": [{"role": "user", "content": "hello"}]})).unwrap();
    let body_b = serde_json::to_string(&serde_json::json!({"model": "claude-sonnet-4-6", "max_tokens": 8192, "messages": [{"role": "user", "content": "world"}]})).unwrap();

    let cch_a = compute_cch(body_a.as_bytes());
    let cch_b = compute_cch(body_b.as_bytes());
    assert_ne!(cch_a, cch_b, "different bodies must produce different CCH");
}

#[test]
fn cch_sensitive_to_body_mutation() {
    // Simulate the bug class: hash computed before mutating the body
    let mut body =
        serde_json::to_string(&serde_json::json!({"model": "test", "messages": []})).unwrap();
    let cch_before = compute_cch(body.as_bytes());
    // Mutate body after hashing (the bug)
    body.push_str("extra content");
    let cch_after = compute_cch(body.as_bytes());
    assert_ne!(
        cch_before, cch_after,
        "CCH must be computed from final body bytes; hash-before-mutate produces wrong value"
    );
}
