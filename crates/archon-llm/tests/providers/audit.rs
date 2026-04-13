//! TC-PROV-audit: include_str!() the openai_compat.rs source and verify
//! the "zero provider-id string branching in shared impl" contract.
//!
//! TASK-AGS-705 quirks move per-provider behavior into `ProviderQuirks`
//! tables. Any regression that reintroduces `if provider_id == "groq"`
//! style branching inside the SHARED impl must fail CI immediately.

const COMPAT_SRC: &str = include_str!("../../src/providers/openai_compat.rs");

#[test]
fn test_no_provider_id_string_branching_in_shared_impl() {
    // Forbidden literal equality branches on provider id. These patterns
    // would indicate the shared impl has been specialized — which is
    // exactly the anti-pattern TASK-AGS-705 eliminated.
    let forbidden = [
        "== \"groq\"",
        "== \"deepseek\"",
        "== \"mistral\"",
        "== \"openrouter\"",
        "== \"fireworks\"",
        "== \"together\"",
    ];

    let mut violations: Vec<&str> = Vec::new();
    for pat in &forbidden {
        if COMPAT_SRC.contains(pat) {
            violations.push(pat);
        }
    }

    assert!(
        violations.is_empty(),
        "openai_compat.rs must not contain provider-id string branching; \
         found forbidden patterns: {:?}. TASK-AGS-705 contract broken.",
        violations
    );
}
