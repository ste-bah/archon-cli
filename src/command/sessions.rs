//! Session management command handlers.

use crate::cli_args::Cli;
use crate::command::utils::parse_datetime;

/// Handle `--sessions` flag: search, stats, or delete sessions.
pub fn handle_sessions(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
) -> anyhow::Result<()> {
    let db_path = crate::command::store_paths::session_db_path(config);
    let store = crate::command::store_paths::open_session_store(&db_path)?;

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
