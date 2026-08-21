use super::*;

const LIVE: &str = "workflow stage failed: your patch would make source file 'crates/archon-trading/src/data_store/data_store_ahdm_tests.rs' 504 lines (currently 495, cap 500); the ENTIRE patch is rejected. Put the new code in a new file under 'crates/archon-trading/src/data_store/data_store_ahdm_tests/'";

#[test]
fn the_live_rejection_is_recognised_and_its_count_read() {
    assert!(is_line_cap_rejection(LIVE));
    assert_eq!(rejected_line_count(LIVE), Some(504));
}

#[test]
fn other_failures_are_not_this_one() {
    assert!(!is_line_cap_rejection(
        "changed files outside declared ownership"
    ));
    assert!(!is_line_cap_rejection("agent transport failed"));
}

/// The live sequence: 512 then 504 is progress and earns another go.
#[test]
fn a_closer_attempt_retries() {
    assert!(should_retry(None, Some(512)));
    assert!(should_retry(Some(512), Some(504)));
}

/// Equal or worse is not progress, so it stops rather than spinning.
#[test]
fn a_stalled_or_worsening_attempt_stops() {
    assert!(!should_retry(Some(504), Some(504)));
    assert!(!should_retry(Some(504), Some(560)));
    assert!(!should_retry(Some(504), None));
}

#[test]
fn the_notice_says_the_whole_patch_was_lost() {
    let notice = retry_notice(LIVE);
    assert!(notice.contains("REJECTED IN FULL"));
    assert!(
        notice.contains("data_store_ahdm_tests/"),
        "carries the remedy"
    );
}
