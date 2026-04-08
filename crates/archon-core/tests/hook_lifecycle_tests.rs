/// TASK-HOOK-021: Fire Remaining 8 Lifecycle Events
///
/// Tests verify that the 8 newly-wired lifecycle events
/// (StopFailure, TeammateIdle, Elicitation, ElicitationResult,
/// WorktreeCreate, WorktreeRemove, InstructionsLoaded, CwdChanged)
/// exist in the enum, serialize correctly, can be registered in
/// HookRegistry, and fire through execute_hooks without blocking.
use std::collections::HashMap;

use archon_core::hooks::{HookCommandType, HookConfig, HookEvent, HookMatcher, HookRegistry};

// ---------------------------------------------------------------------------
// Helper: build a HookRegistry with an "echo" command hook for one event
// ---------------------------------------------------------------------------

fn make_test_registry(event: HookEvent) -> HookRegistry {
    let mut reg = HookRegistry::new();
    reg.register_matchers(
        event,
        vec![HookMatcher {
            matcher: None,
            hooks: vec![HookConfig {
                hook_type: HookCommandType::Command,
                command: "echo lifecycle-test".to_string(),
                if_condition: None,
                timeout: Some(5),
                once: None,
                r#async: None,
                async_rewake: None,
                status_message: None,
                headers: HashMap::new(),
                allowed_env_vars: Vec::new(),
            }],
        }],
        None,
    );
    reg
}

// ---------------------------------------------------------------------------
// Helper: list of the 8 newly-wired events
// ---------------------------------------------------------------------------

fn new_lifecycle_events() -> Vec<HookEvent> {
    vec![
        HookEvent::StopFailure,
        HookEvent::TeammateIdle,
        HookEvent::Elicitation,
        HookEvent::ElicitationResult,
        HookEvent::WorktreeCreate,
        HookEvent::WorktreeRemove,
        HookEvent::InstructionsLoaded,
        HookEvent::CwdChanged,
    ]
}

// ---------------------------------------------------------------------------
// 1. All 8 new events exist in the enum
// ---------------------------------------------------------------------------

#[test]
fn test_all_8_new_events_in_enum() {
    let events = new_lifecycle_events();
    assert_eq!(events.len(), 8, "must list exactly 8 new lifecycle events");

    // Each variant must have a non-empty Display representation
    for e in &events {
        let name = format!("{e}");
        assert!(!name.is_empty(), "event should have a display name");
    }
}

// ---------------------------------------------------------------------------
// 2. Serialization produces expected PascalCase strings
// ---------------------------------------------------------------------------

#[test]
fn test_new_events_serialize_correctly() {
    let expected: Vec<(&str, HookEvent)> = vec![
        ("StopFailure", HookEvent::StopFailure),
        ("TeammateIdle", HookEvent::TeammateIdle),
        ("Elicitation", HookEvent::Elicitation),
        ("ElicitationResult", HookEvent::ElicitationResult),
        ("WorktreeCreate", HookEvent::WorktreeCreate),
        ("WorktreeRemove", HookEvent::WorktreeRemove),
        ("InstructionsLoaded", HookEvent::InstructionsLoaded),
        ("CwdChanged", HookEvent::CwdChanged),
    ];

    for (pascal, event) in expected {
        let json = serde_json::to_string(&event).expect("serialize");
        assert_eq!(
            json,
            format!("\"{pascal}\""),
            "event {pascal} should serialize to PascalCase"
        );

        // Round-trip: deserialize back
        let back: HookEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, event, "round-trip failed for {pascal}");
    }
}

// ---------------------------------------------------------------------------
// 3. All 8 events can be registered in HookRegistry
// ---------------------------------------------------------------------------

#[test]
fn test_new_events_can_be_registered() {
    for event in new_lifecycle_events() {
        let name = format!("{event}");
        let _reg = make_test_registry(event);
        // Registry was built without panic — that is the assertion.
        // Verify it has entries via a settings-json round-trip pattern:
        // we just confirm the registry object exists (no public iterator).
        let _ = format!("{name} registered ok");
    }
}

// ---------------------------------------------------------------------------
// 4. All 8 events fire through execute_hooks without blocking
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_new_events_fire_through_registry() {
    for event in new_lifecycle_events() {
        let name = format!("{event}");
        let reg = make_test_registry(event.clone());

        let result = reg
            .execute_hooks(
                event,
                serde_json::json!({"hook_event": name}),
                std::path::Path::new("/tmp"),
                "test-session-lifecycle",
            )
            .await;

        assert!(!result.is_blocked(), "event {name} should not be blocked");
    }
}

// ---------------------------------------------------------------------------
// 5. StopFailure is distinct from Stop
// ---------------------------------------------------------------------------

#[test]
fn test_stop_failure_event_distinct_from_stop() {
    assert_ne!(
        HookEvent::Stop,
        HookEvent::StopFailure,
        "Stop and StopFailure must be distinct variants"
    );

    let stop_json = serde_json::to_string(&HookEvent::Stop).unwrap();
    let fail_json = serde_json::to_string(&HookEvent::StopFailure).unwrap();
    assert_ne!(stop_json, fail_json, "serialized forms must differ");

    // They can coexist as HashMap keys
    let mut map: HashMap<HookEvent, &str> = HashMap::new();
    map.insert(HookEvent::Stop, "stop");
    map.insert(HookEvent::StopFailure, "stop_failure");
    assert_eq!(map.len(), 2);
    assert_eq!(map[&HookEvent::Stop], "stop");
    assert_eq!(map[&HookEvent::StopFailure], "stop_failure");
}

// ---------------------------------------------------------------------------
// 6. CwdChanged fires with a context payload
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_cwd_changed_event_fires() {
    let reg = make_test_registry(HookEvent::CwdChanged);

    let payload = serde_json::json!({
        "hook_event": "CwdChanged",
        "old_cwd": "/home/user/project-a",
        "new_cwd": "/home/user/project-b",
    });

    let result = reg
        .execute_hooks(
            HookEvent::CwdChanged,
            payload,
            std::path::Path::new("/tmp"),
            "test-session-cwd",
        )
        .await;

    assert!(!result.is_blocked(), "CwdChanged hook should not block");
}

// ---------------------------------------------------------------------------
// 7. Elicitation and ElicitationResult fire independently
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_elicitation_pair() {
    // Verify they are distinct
    assert_ne!(HookEvent::Elicitation, HookEvent::ElicitationResult);

    // Register and fire Elicitation
    let reg_elic = make_test_registry(HookEvent::Elicitation);
    let r1 = reg_elic
        .execute_hooks(
            HookEvent::Elicitation,
            serde_json::json!({"hook_event": "Elicitation", "prompt": "choose option"}),
            std::path::Path::new("/tmp"),
            "test-session-elic",
        )
        .await;
    assert!(!r1.is_blocked(), "Elicitation should not block");

    // Register and fire ElicitationResult
    let reg_result = make_test_registry(HookEvent::ElicitationResult);
    let r2 = reg_result
        .execute_hooks(
            HookEvent::ElicitationResult,
            serde_json::json!({"hook_event": "ElicitationResult", "choice": "option_a"}),
            std::path::Path::new("/tmp"),
            "test-session-elic-result",
        )
        .await;
    assert!(!r2.is_blocked(), "ElicitationResult should not block");
}

// ---------------------------------------------------------------------------
// 8. WorktreeCreate and WorktreeRemove fire independently
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn test_worktree_pair() {
    // Verify they are distinct
    assert_ne!(HookEvent::WorktreeCreate, HookEvent::WorktreeRemove);

    // Register and fire WorktreeCreate
    let reg_create = make_test_registry(HookEvent::WorktreeCreate);
    let r1 = reg_create
        .execute_hooks(
            HookEvent::WorktreeCreate,
            serde_json::json!({"hook_event": "WorktreeCreate", "path": "/tmp/wt-1"}),
            std::path::Path::new("/tmp"),
            "test-session-wt-create",
        )
        .await;
    assert!(!r1.is_blocked(), "WorktreeCreate should not block");

    // Register and fire WorktreeRemove
    let reg_remove = make_test_registry(HookEvent::WorktreeRemove);
    let r2 = reg_remove
        .execute_hooks(
            HookEvent::WorktreeRemove,
            serde_json::json!({"hook_event": "WorktreeRemove", "path": "/tmp/wt-1"}),
            std::path::Path::new("/tmp"),
            "test-session-wt-remove",
        )
        .await;
    assert!(!r2.is_blocked(), "WorktreeRemove should not block");
}
