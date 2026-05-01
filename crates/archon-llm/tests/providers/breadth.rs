//! TC-PROV-01: registry breadth invariant. GHOST-003: 4 stub providers
//! removed — contract is now at least 36 total (31 compat + 5 native).
//! This test fails LOUDLY if any future patch accidentally removes a
//! registry entry.

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
        native >= 5,
        "NATIVE_REGISTRY must contain at least 5 entries; got {native}"
    );
    assert!(
        total >= 36,
        "total provider count must be >= 36 (TC-PROV-01); got {total}"
    );
}
