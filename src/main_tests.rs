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
    let result = run_kb_with_temp_store(cli_args::KbAction::List { kb: None }).await;
    assert!(result.is_ok(), "list on empty DB must succeed");
}

#[tokio::test]
async fn kb_search_on_empty_db_returns_no_matches() {
    let result = run_kb_with_temp_store(cli_args::KbAction::Search {
        query: "nonexistent".into(),
        limit: 10,
        mode: "exact".into(),
        kb: None,
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
    // The temporary store is passed as an argument. This used to go through
    // `ARCHON_KB_DB_PATH` under a comment claiming `--test-threads=1`; nothing pins
    // the thread count, and these tests share a process with the whole bin target (#166).
    crate::command::kb::handle_kb_command_at(&db_path, action).await
}

#[tokio::test]
async fn startup_read_only_subcommand_never_sets_up_voice() {
    assert_subcommand_skips_voice(&["workflow", "decomposition-identity"], None).await;
}

#[tokio::test]
async fn startup_refused_decompose_never_sets_up_voice() {
    assert_subcommand_skips_voice(
        &[
            "workflow",
            "decompose",
            "--prd",
            "missing.md",
            "--tasks",
            "missing-tasks",
        ],
        Some("requires --yes"),
    )
    .await;
}

#[tokio::test]
async fn startup_invalid_freeze_host_call_never_sets_up_voice() {
    assert_subcommand_skips_voice(
        &[
            "workflow",
            "freeze-acceptance",
            "--prd",
            "missing.md",
            "--tasks",
            "missing-tasks",
            "--candidate-stdin",
        ],
        Some("--staging-root"),
    )
    .await;
}

async fn assert_subcommand_skips_voice(args: &[&str], expected_error: Option<&str>) {
    let cli = Cli::try_parse_from(std::iter::once("archon").chain(args.iter().copied())).unwrap();
    let mut config = archon_core::config::ArchonConfig::default();
    config.voice.enabled = true;
    let env_vars = archon_core::env_vars::load_env_vars_from(&Default::default());
    let flags = archon_core::cli_flags::ResolvedFlags::default();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let calls = std::cell::Cell::new(0);
    let outcome = super::main_startup::run(
        super::dispatch_modes(cli, &config, &env_vars, &flags, "test-session", &cwd),
        || async { calls.set(calls.get() + 1) },
        |_, ()| async { panic!("subcommand reached interactive session") },
    )
    .await;
    match expected_error {
        Some(message) => assert!(outcome.unwrap_err().to_string().contains(message)),
        None => outcome.unwrap(),
    }
    assert_eq!(calls.get(), 0, "subcommand started voice setup");
}
