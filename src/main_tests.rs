use anyhow::Result;
use clap::Parser;
use serde_json::json;

use crate::cli_args::{self, Cli};

#[test]
fn json_schema_path_reads_schema_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema.json");
    let schema = r#"{"type":"object","required":["ok"]}"#;
    std::fs::write(&path, schema).unwrap();
    let cli = Cli::try_parse_from([
        "archon",
        "-p",
        "return json",
        "--json-schema-path",
        path.to_str().unwrap(),
    ])
    .unwrap();

    assert_eq!(
        super::resolve_json_schema(&cli).unwrap(),
        Some(schema.to_string())
    );
}

#[test]
fn strip_cache_control_noop_when_enabled() {
    let mut blocks = vec![
        json!({"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}),
        json!({"type": "text", "text": "b"}),
    ];
    crate::setup::strip_cache_control_if_disabled(&mut blocks, true);
    assert!(blocks[0].get("cache_control").is_some());
    assert!(blocks[1].get("cache_control").is_none());
}

#[test]
fn strip_cache_control_removes_key_when_disabled() {
    let mut blocks = vec![
        json!({"type": "text", "text": "a", "cache_control": {"type": "ephemeral"}}),
        json!({"type": "text", "text": "b", "cache_control": {"type": "ephemeral", "scope": "org"}}),
        json!({"type": "text", "text": "c"}),
    ];
    crate::setup::strip_cache_control_if_disabled(&mut blocks, false);
    assert!(blocks[0].get("cache_control").is_none());
    assert!(blocks[1].get("cache_control").is_none());
    assert!(blocks[2].get("cache_control").is_none());
    assert_eq!(blocks[0].get("text").unwrap(), "a");
    assert_eq!(blocks[1].get("text").unwrap(), "b");
    assert_eq!(blocks[2].get("text").unwrap(), "c");
}

#[tokio::test]
async fn kb_stats_on_empty_db() {
    let result = run_kb_with_temp_store(cli_args::KbAction::Stats).await;
    assert!(result.is_ok(), "stats on empty DB must succeed");
}

#[tokio::test]
async fn kb_list_on_empty_db() {
    let result = run_kb_with_temp_store(cli_args::KbAction::List).await;
    assert!(result.is_ok(), "list on empty DB must succeed");
}

#[tokio::test]
async fn kb_search_on_empty_db_returns_no_matches() {
    let result = run_kb_with_temp_store(cli_args::KbAction::Search {
        query: "nonexistent".into(),
        limit: 10,
        mode: "exact".into(),
    })
    .await;
    assert!(result.is_ok(), "search on empty DB must succeed");
}

#[tokio::test]
async fn kb_stats_default_subcommand_works() {
    let result = run_kb_with_temp_store(cli_args::KbAction::Stats).await;
    assert!(result.is_ok());
}

async fn run_kb_with_temp_store(action: cli_args::KbAction) -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("kb.db");
    // Tests run with --test-threads=1 in this workstream, so mutating this
    // process environment cannot race another KB test.
    unsafe {
        std::env::set_var("ARCHON_KB_DB_PATH", &db_path);
    }
    let result = crate::command::kb::handle_kb_command(action).await;
    unsafe {
        std::env::remove_var("ARCHON_KB_DB_PATH");
    }
    result
}
