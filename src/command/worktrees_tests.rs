//! `/worktrees` tests (#184 M7).
//!
//! The listing reads a user-global directory, so these cover the pure parts —
//! argument handling, age rendering, and the usage text — rather than
//! manufacturing worktrees in a shared location. The end-to-end merge/discard
//! behaviour is covered by `worktree_tests.rs` against a real repository.

use super::*;

#[test]
fn acting_without_an_owner_explains_itself() {
    let message = act(&[], ExitAction::Merge, "merge", "session-test");
    assert!(message.contains("missing <owner>"), "{message}");
    assert!(message.contains("/worktrees merge"), "{message}");
}

#[test]
fn acting_on_an_unknown_owner_says_so_rather_than_failing_silently() {
    let message = act(
        &["subagent-does-not-exist".to_string()],
        ExitAction::Discard,
        "discard",
        "session-test",
    );
    assert!(message.contains("no worktree owned by"), "{message}");
    assert!(
        message.contains("subagent-does-not-exist"),
        "the message should name the owner that was not found: {message}"
    );
}

#[test]
fn the_usage_text_lists_every_action() {
    let text = usage("test");
    for action in ["merge", "discard", "keep", "prune", "sizes"] {
        assert!(
            text.contains(action),
            "usage should mention {action}: {text}"
        );
    }
}

/// Ages are read at a glance, so the unit changes with the magnitude rather
/// than reporting 4,320 minutes.
#[test]
fn age_renders_in_the_largest_useful_unit() {
    assert_eq!(humanise_age(chrono::Duration::minutes(5)), "5m");
    assert_eq!(humanise_age(chrono::Duration::minutes(59)), "59m");
    assert_eq!(humanise_age(chrono::Duration::hours(3)), "3h");
    assert_eq!(humanise_age(chrono::Duration::hours(47)), "47h");
    assert_eq!(humanise_age(chrono::Duration::days(3)), "3d");
}

/// Sizing walks every file under a `target/`, which is gigabytes across
/// hundreds of thousands of files, and this handler is synchronous on the
/// dispatch path. The plain listing must not pay that.
#[test]
fn the_plain_listing_offers_sizes_rather_than_computing_them() {
    let listing = render_list(false);
    if listing.contains("No agent worktrees") {
        return;
    }
    assert!(listing.contains("/worktrees sizes"), "{listing}");
}

#[test]
fn an_empty_listing_says_so_plainly() {
    // Only meaningful when the developer's own tree is empty; when it is not,
    // the assertion below still holds for whatever is there.
    let listing = render_list(false);
    assert!(
        listing.contains("No agent worktrees") || listing.contains("agent worktree(s)"),
        "{listing}"
    );
}
