//! Tests for the model-callable session search (#189 Phase 2).

use super::*;
use crate::tool::AgentMode;

fn ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

#[test]
fn metadata_names_the_tool_and_takes_no_required_argument() {
    let tool = SessionSearchTool::default();

    assert_eq!(tool.name(), "SessionSearch");
    let schema = tool.input_schema();
    assert_eq!(
        schema["required"].as_array().map(Vec::len),
        Some(0),
        "every filter is optional; a bare call lists recent sessions"
    );
    assert!(schema["properties"]["query"].is_object());
    assert!(schema["properties"]["limit"].is_object());
}

/// The description is what decides whether the model ever calls this. It has to
/// name the situations, not the subsystem.
#[test]
fn the_description_names_when_to_reach_for_it() {
    let description = SessionSearchTool::default().description().to_lowercase();

    assert!(description.contains("use when"), "{description}");
    assert!(description.contains("earlier"), "{description}");
}

#[test]
fn reading_local_session_data_is_safe() {
    assert_eq!(
        SessionSearchTool::default().permission_level(&json!({"query": "x"})),
        PermissionLevel::Safe
    );
    assert_eq!(
        SessionSearchTool::default().working_tree_effect(),
        WorkingTreeEffect::None
    );
}

#[test]
fn the_limit_is_defaulted_and_clamped() {
    assert_eq!(requested_limit(&json!({})), DEFAULT_LIMIT);
    assert_eq!(requested_limit(&json!({"limit": 3})), 3);
    assert_eq!(requested_limit(&json!({"limit": 9_999})), MAX_LIMIT);
    assert_eq!(
        requested_limit(&json!({"limit": 0})),
        1,
        "zero would ask for nothing and read as a broken search"
    );
}

/// Silently ignoring an unparseable date would answer a different question than
/// the one asked, and an empty result would look legitimate.
#[tokio::test]
async fn an_unparseable_date_is_refused_rather_than_ignored() {
    let result = SessionSearchTool::default()
        .execute(json!({"query": "x", "after": "last tuesday"}), &ctx())
        .await;

    assert!(result.is_error);
    assert!(result.content.contains("RFC3339"), "{}", result.content);
}

#[test]
fn timestamps_parse_when_well_formed() {
    let parsed = timestamp_arg(&json!({"after": "2026-01-02T03:04:05Z"}), "after")
        .expect("valid RFC3339 parses");
    assert!(parsed.is_some());
    assert_eq!(timestamp_arg(&json!({}), "after"), Ok(None));
}

#[test]
fn blank_arguments_are_treated_as_absent() {
    assert_eq!(string_arg(&json!({"query": "   "}), "query"), None);
    assert_eq!(
        string_arg(&json!({"query": " hi "}), "query"),
        Some("hi".into())
    );
    assert_eq!(string_arg(&json!({}), "query"), None);
}

#[test]
fn an_excerpt_is_bounded_and_never_splits_a_character() {
    let long = "é".repeat(1_000);
    let cut = truncate(&long, MAX_EXCERPT_CHARS);

    assert!(
        cut.chars().count() <= MAX_EXCERPT_CHARS + 1,
        "plus the ellipsis"
    );
    assert!(cut.ends_with('…'));
    assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
}

#[test]
fn a_short_excerpt_is_returned_whole_without_an_ellipsis() {
    assert_eq!(truncate("short line", MAX_EXCERPT_CHARS), "short line");
}

#[test]
fn excerpt_whitespace_is_collapsed_so_one_hit_stays_one_line() {
    assert_eq!(truncate("a\n\n  b\tc", MAX_EXCERPT_CHARS), "a b c");
}

/// A broad query can match months of sessions. A tool added to relieve context
/// pressure must not become a way to cause it.
#[test]
fn a_large_result_set_is_bounded_and_says_what_it_dropped() {
    let rows: Vec<serde_json::Value> = (0..400)
        .map(|i| {
            json!({
                "session_id": format!("session-{i}"),
                "excerpt": "x".repeat(300),
                "working_directory": "/very/long/path/that/repeats".repeat(4),
            })
        })
        .collect();

    let result = render(&rows);

    assert!(!result.is_error);
    assert!(
        result.content.len() <= MAX_RESPONSE_BYTES,
        "payload was {} bytes",
        result.content.len()
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&result.content).expect("a bounded payload must still parse");
    assert!(
        parsed["omitted"].as_u64().unwrap_or(0) > 0,
        "dropping rows silently would misreport the result set"
    );
}

#[test]
fn a_small_result_set_is_returned_whole_with_no_omission_note() {
    let rows = vec![json!({"session_id": "s1"}), json!({"session_id": "s2"})];

    let result = render(&rows);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).expect("parses");

    assert_eq!(parsed["matches"].as_array().map(Vec::len), Some(2));
    assert!(parsed.get("omitted").is_none());
}

/// The env override, the configured path and the default must resolve in the
/// same order the bin crate's `session_db_path` uses, or the tool and
/// `/sessions` would read different databases.
#[test]
fn the_configured_path_is_used_when_no_environment_override_is_set() {
    let configured = PathBuf::from("/configured/sessions.db");
    let tool = SessionSearchTool::new(Some(configured.clone()));

    if std::env::var_os("ARCHON_SESSION_DB_PATH").is_some() {
        // Another test in this process set the override; the precedence rule
        // is still what it is, and asserting against a value we did not set
        // would be asserting about that test instead.
        return;
    }
    assert_eq!(tool.db_path(), configured);
}

#[test]
fn the_default_location_is_used_when_nothing_is_configured() {
    if std::env::var_os("ARCHON_SESSION_DB_PATH").is_some() {
        return;
    }
    assert_eq!(
        SessionSearchTool::default().db_path(),
        archon_session::storage::default_db_path()
    );
}

/// A missing store is a plain answer, not a panic — a fresh checkout has no
/// session history and calling this there must not look like a crash.
#[tokio::test]
async fn a_missing_database_reports_rather_than_panicking() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tool = SessionSearchTool::new(Some(dir.path().join("nested/absent.db")));

    let result = tool.execute(json!({"query": "anything"}), &ctx()).await;

    // Either outcome is acceptable; a panic is not.
    assert!(result.is_error || result.content.contains("No matching sessions"));
}
