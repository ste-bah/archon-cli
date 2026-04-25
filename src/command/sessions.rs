//! Session management command handlers.
//! Extracted from main.rs to reduce main.rs from 6234 to < 500 lines.

use crate::cli_args::Cli;
use crate::command::utils::parse_datetime;

/// Handle `--sessions` flag: search, stats, or delete sessions.
pub fn handle_sessions(cli: &Cli) -> anyhow::Result<()> {
    let db_path = archon_session::storage::default_db_path();
    let store = archon_session::storage::SessionStore::open(&db_path)
        .map_err(|e| anyhow::anyhow!("failed to open session database: {e}"))?;

    // --sessions --delete <ID>
    if let Some(ref id) = cli.delete {
        store
            .delete_session(id)
            .map_err(|e| anyhow::anyhow!("failed to delete session: {e}"))?;
        eprintln!("Deleted session {id}");
        return Ok(());
    }

    // --sessions --stats
    if cli.stats {
        let stats = archon_session::search::session_stats(&store)
            .map_err(|e| anyhow::anyhow!("failed to compute stats: {e}"))?;
        println!("Sessions:  {}", stats.total_sessions);
        println!("Tokens:    {}", stats.total_tokens);
        println!("Messages:  {}", stats.total_messages);
        println!("Avg dur:   {:.0}s", stats.avg_duration_secs);
        return Ok(());
    }

    // Build search query from CLI flags.
    let after = cli.after.as_ref().map(|s| parse_datetime(s)).transpose()?;
    let before = cli.before.as_ref().map(|s| parse_datetime(s)).transpose()?;

    let query = archon_session::search::SessionSearchQuery {
        branch: cli.branch.clone(),
        directory: cli.session_dir.clone(),
        after,
        before,
        text: cli.search.clone(),
        tag: None,
        ..Default::default()
    };

    let results = archon_session::search::search_sessions(&store, &query)
        .map_err(|e| anyhow::anyhow!("search failed: {e}"))?;

    if results.is_empty() {
        eprintln!("No matching sessions found.");
    } else {
        for session in &results {
            println!("{}", archon_session::resume::format_session_line(session));
        }
    }

    Ok(())
}
