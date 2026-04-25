//! Smoke tests for SessionBrowser::restore public API.
//!
//! Exercises the three ResumeOutcome paths:
//! - Test 1: Restored — session fits within generous context
//! - Test 2: ContextOverflow — session exceeds context, TruncateOldest returned
//! - Test 3: NotFound — unknown session id returns NotFound

use archon_tui::screens::session_browser::{OverflowAction, ResumeOutcome, SessionBrowser};
use std::sync::Arc;

/// Test 1: Restored path — session fits within generous context limit.
/// Verifies that when estimated tokens are well under the limit,
/// restore returns ResumeOutcome::Restored with correct session_id and message count.
#[tokio::test]
async fn smoke_restore_fits_within_context() {
    // Create a browser with an in-memory test store.
    // browser holds temp_dir which keeps the DB alive.
    let browser = SessionBrowser::new_for_tests();
    let store = browser.store();

    // Register a session with 5 short messages
    let session_id = "smoke-restore-fits";
    store
        .register_session(session_id, "/tmp", None, "claude-3-5-sonnet")
        .expect("register session");
    for i in 0..5 {
        store
            .save_message(session_id, i, &format!("Message content {}", i))
            .expect("save message");
    }

    // Create a fresh browser and restore with generous context limit.
    // browser2 shares the store with browser, so temp_dir stays alive.
    let mut browser2 = SessionBrowser::new(Arc::clone(&store));
    let outcome = browser2
        .restore(session_id, 100_000)
        .await
        .expect("restore should succeed");

    // Verify Restored outcome
    match outcome {
        ResumeOutcome::Restored {
            session_id: restored_sid,
            messages_loaded,
        } => {
            assert_eq!(restored_sid, session_id, "session_id must match");
            assert_eq!(messages_loaded, 5, "all 5 messages should be loaded");
        }
        other => panic!("Expected Restored, got {:?}", other),
    }

    // Verify state was updated via state() accessor
    assert_eq!(
        browser2.state().current_id,
        Some(session_id.to_string()),
        "state.current_id should be set after restore"
    );

    // browser stays in scope here to keep temp_dir alive for browser2's store
    drop(browser);
    drop(store);
}

/// Test 2: ContextOverflow path — session exceeds context limit.
/// Verifies that when estimated tokens exceed the limit,
/// restore returns ResumeOutcome::ContextOverflow with TruncateOldest action.
#[tokio::test]
async fn smoke_restore_exceeds_context_truncates() {
    // Create a browser with an in-memory test store.
    let browser = SessionBrowser::new_for_tests();
    let store = browser.store();

    // Register a session with 100 messages — each ~50 chars (~12-13 tokens via chars/4)
    // Total: ~1200 tokens, well over the limit of 100
    let session_id = "smoke-restore-overflow";
    store
        .register_session(session_id, "/tmp", None, "claude-3-5-sonnet")
        .expect("register session");
    for i in 0..100 {
        let content = format!(
            "Message number {} with some additional text to make it longer",
            i
        );
        store
            .save_message(session_id, i, &content)
            .expect("save message");
    }

    // Create a fresh browser and restore with tiny context limit.
    let mut browser2 = SessionBrowser::new(Arc::clone(&store));
    let outcome = browser2
        .restore(session_id, 100)
        .await
        .expect("restore should succeed");

    // Verify ContextOverflow outcome
    match outcome {
        ResumeOutcome::ContextOverflow {
            estimated_tokens: _,
            limit,
            action,
        } => {
            assert_eq!(limit, 100, "limit should match the tight context limit");
            match action {
                OverflowAction::TruncateOldest(n) => {
                    assert!(n > 0, "TruncateOldest count must be non-zero, got {}", n);
                }
                other => panic!("Expected TruncateOldest, got {:?}", other),
            }
        }
        other => panic!("Expected ContextOverflow, got {:?}", other),
    }

    drop(browser);
    drop(store);
}

/// Test 3: NotFound path — unknown session id returns NotFound.
/// Verifies that attempting to restore a non-existent session
/// returns ResumeOutcome::NotFound without error.
#[tokio::test]
async fn smoke_restore_unknown_session_returns_not_found() {
    // Create a browser with an in-memory test store.
    let browser = SessionBrowser::new_for_tests();
    let store = browser.store();

    // Create a fresh browser and try to restore a non-existent session.
    let mut browser2 = SessionBrowser::new(Arc::clone(&store));
    let outcome = browser2
        .restore("nonexistent-session-id", 100_000)
        .await
        .expect("restore should succeed (NotFound is not an error)");

    // Verify NotFound outcome
    match outcome {
        ResumeOutcome::NotFound => {}
        other => panic!("Expected NotFound, got {:?}", other),
    }

    drop(browser);
    drop(store);
}
