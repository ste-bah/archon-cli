//! Integration tests for hook callback/plugin registration API.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_core::hooks::{
    HookCallback, HookCallbackEntry, HookContext, HookEvent, HookRegistry, HookResult,
    SourceAuthority,
};

#[tokio::test]
async fn test_register_callback_fires_on_event() {
    let registry = HookRegistry::new();
    let fired = Arc::new(AtomicBool::new(false));
    let fired_clone = fired.clone();

    let cb: HookCallback = Arc::new(move |_ctx: &HookContext| {
        fired_clone.store(true, Ordering::SeqCst);
        HookResult::allow()
    });

    registry.register_callback(
        HookEvent::PreToolUse,
        HookCallbackEntry {
            name: "test-cb".to_string(),
            callback: cb,
            authority: SourceAuthority::User,
            timeout_secs: 5,
        },
    );

    let _result = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-1",
        )
        .await;

    assert!(fired.load(Ordering::SeqCst), "callback should have fired");
}

#[tokio::test]
async fn test_callback_result_merged_into_aggregate() {
    let registry = HookRegistry::new();

    let cb: HookCallback = Arc::new(|_ctx: &HookContext| {
        let mut result = HookResult::allow();
        result.additional_context = Some("extra-info-from-callback".to_string());
        result
    });

    registry.register_callback(
        HookEvent::PreToolUse,
        HookCallbackEntry {
            name: "ctx-cb".to_string(),
            callback: cb,
            authority: SourceAuthority::User,
            timeout_secs: 5,
        },
    );

    let agg = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-2",
        )
        .await;

    assert!(
        agg.additional_contexts.contains(&"extra-info-from-callback".to_string()),
        "additional_context should be merged into aggregate"
    );
}

#[tokio::test]
async fn test_callback_panic_caught_safely() {
    let registry = HookRegistry::new();

    let cb: HookCallback = Arc::new(|_ctx: &HookContext| {
        panic!("intentional panic in callback");
    });

    registry.register_callback(
        HookEvent::PreToolUse,
        HookCallbackEntry {
            name: "panic-cb".to_string(),
            callback: cb,
            authority: SourceAuthority::User,
            timeout_secs: 5,
        },
    );

    // Should not crash -- panics are caught.
    let agg = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-3",
        )
        .await;

    // Result should be default Success (no blocking errors).
    assert!(
        !agg.is_blocked(),
        "panic should not produce a blocking error"
    );
}

#[tokio::test]
async fn test_callback_timeout() {
    let registry = HookRegistry::new();

    let cb: HookCallback = Arc::new(|_ctx: &HookContext| {
        // Block for 5 seconds -- should be interrupted by 1s timeout.
        std::thread::sleep(Duration::from_secs(5));
        HookResult::allow()
    });

    registry.register_callback(
        HookEvent::PreToolUse,
        HookCallbackEntry {
            name: "slow-cb".to_string(),
            callback: cb,
            authority: SourceAuthority::User,
            timeout_secs: 1,
        },
    );

    let start = Instant::now();
    let _agg = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-4",
        )
        .await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(3),
        "should timeout in ~1s, not block for 5s (elapsed: {:?})",
        elapsed
    );
}

#[tokio::test]
async fn test_unregister_callback() {
    let registry = HookRegistry::new();
    let fired = Arc::new(AtomicBool::new(false));
    let fired_clone = fired.clone();

    let cb: HookCallback = Arc::new(move |_ctx: &HookContext| {
        fired_clone.store(true, Ordering::SeqCst);
        HookResult::allow()
    });

    registry.register_callback(
        HookEvent::PreToolUse,
        HookCallbackEntry {
            name: "removable-cb".to_string(),
            callback: cb,
            authority: SourceAuthority::User,
            timeout_secs: 5,
        },
    );

    // Unregister before firing.
    registry.unregister_callback(&HookEvent::PreToolUse, "removable-cb");

    let _agg = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-5",
        )
        .await;

    assert!(
        !fired.load(Ordering::SeqCst),
        "callback should NOT fire after unregister"
    );
}

#[tokio::test]
async fn test_multiple_callbacks_all_fire() {
    let registry = HookRegistry::new();
    let counter = Arc::new(AtomicU32::new(0));

    for i in 0..3 {
        let counter_clone = counter.clone();
        let cb: HookCallback = Arc::new(move |_ctx: &HookContext| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
            let mut result = HookResult::allow();
            result.additional_context = Some(format!("cb-{i}"));
            result
        });

        registry.register_callback(
            HookEvent::PreToolUse,
            HookCallbackEntry {
                name: format!("multi-cb-{i}"),
                callback: cb,
                authority: SourceAuthority::User,
                timeout_secs: 5,
            },
        );
    }

    let agg = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "sess-6",
        )
        .await;

    assert_eq!(counter.load(Ordering::SeqCst), 3, "all 3 callbacks should fire");
    assert_eq!(
        agg.additional_contexts.len(),
        3,
        "all 3 additional_contexts should be merged"
    );
}

#[tokio::test]
async fn test_callback_receives_hook_context() {
    let registry = HookRegistry::new();
    let received_session = Arc::new(std::sync::Mutex::new(String::new()));
    let received_clone = received_session.clone();

    let cb: HookCallback = Arc::new(move |ctx: &HookContext| {
        let mut guard = received_clone.lock().unwrap();
        *guard = ctx.session_id.clone();
        HookResult::allow()
    });

    registry.register_callback(
        HookEvent::PreToolUse,
        HookCallbackEntry {
            name: "ctx-check-cb".to_string(),
            callback: cb,
            authority: SourceAuthority::User,
            timeout_secs: 5,
        },
    );

    let _agg = registry
        .execute_hooks(
            HookEvent::PreToolUse,
            serde_json::json!({}),
            std::path::Path::new("/tmp"),
            "my-session-42",
        )
        .await;

    let received = received_session.lock().unwrap().clone();
    assert_eq!(
        received, "my-session-42",
        "callback should receive the correct session_id via HookContext"
    );
}
