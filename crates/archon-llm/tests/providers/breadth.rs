//! TC-PROV-01: registry breadth invariant. The phase-7 contract commits
//! to at least 40 total providers (31 compat + 9 native). This test
//! fails LOUDLY if any future patch accidentally removes a registry
//! entry — which is the entire point of a CI-enforced forensic contract.

use archon_llm::providers::{count_compat, count_native};

#[test]
fn test_registry_breadth_invariant() {
    let compat = count_compat();
    let native = count_native();
    let total = compat + native;

    assert_eq!(
        compat, 31,
        "OPENAI_COMPAT_REGISTRY must contain exactly 31 entries; got {compat}"
    );
    assert!(
        native >= 9,
        "NATIVE_REGISTRY must contain at least 9 entries; got {native}"
    );
    assert!(
        total >= 40,
        "total provider count must be >= 40 (TC-PROV-01); got {total}"
    );
}
