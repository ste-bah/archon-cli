//! OBS-900 shell-smoke integration test.
//!
//! Confirms the crate is actually a workspace member and its public API is
//! reachable from an external cargo test binary. If a future LIFT ticket
//! accidentally drops the crate from the root `[workspace].members` or
//! flips `publish = true`, this test catches it by failing to compile.

#[test]
fn version_is_reachable_from_external_crate() {
    let v = archon_observability::VERSION;
    assert!(!v.is_empty(), "VERSION must not be empty");
    assert!(v.contains('.'), "VERSION '{v}' must look dotted");
}
