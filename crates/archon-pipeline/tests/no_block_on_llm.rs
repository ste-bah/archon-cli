//! Verify the `block_on_llm` anti-pattern is fully removed from facade.rs.
//!
//! This is a standalone integration test so `include_str!` doesn't self-match.

#[test]
fn test_no_block_on_llm_in_facade() {
    let source = include_str!("../src/gametheory/facade.rs");
    let count = source.matches("fn block_on_llm").count();
    assert_eq!(
        count, 0,
        "fn block_on_llm must not appear in facade.rs, found {count} occurrence(s)"
    );
}
