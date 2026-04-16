//! Background session handlers (CLI-221).
//! Extracted from main.rs to reduce main.rs from 6234 to < 500 lines.

use crate::cli_args::Cli;

/// List background sessions and exit.
pub fn handle_bg_list() -> anyhow::Result<()> {
    // Clean stale PIDs first
    let _ = archon_session::registry::cleanup_stale_pids();

    let sessions =
        archon_session::registry::list_sessions()
            .map_err(|e| anyhow::anyhow!("failed to list background sessions: {e}"))?;

    if sessions.is_empty() {
        eprintln!("No background sessions found.");
    } else {
        eprintln!(
            "{:<10} {:<14} {:<20} {:<8} STARTED",
            "ID", "STATUS", "NAME", "TURNS"
        );
        for s in &sessions {
            let short_id = if s.id.len() > 8 { &s.id[..8] } else { &s.id };
            eprintln!(
                "{:<10} {:<14} {:<20} {:<8} {}",
                short_id, s.status, s.name, s.turns, s.started_at,
            );
        }
    }
    Ok(())
}

/// Kill a background session and exit.
#[cfg(unix)]
pub fn handle_bg_kill(id: &str) -> anyhow::Result<()> {
    archon_session::background::kill_session(id)
        .map_err(|e| anyhow::anyhow!("failed to kill session {id}: {e}"))?;
    eprintln!("Session {id} killed.");
    Ok(())
}

#[cfg(not(unix))]
pub fn handle_bg_kill(_id: &str) -> anyhow::Result<()> {
    eprintln!("Background sessions are only supported on Unix systems.");
    std::process::exit(1);
}

/// Attach to a running background session (stream logs).
#[cfg(unix)]
pub fn handle_bg_attach(id: &str) -> anyhow::Result<()> {
    archon_session::attach::stream_logs(id, true)
        .map_err(|e| anyhow::anyhow!("failed to attach to session {id}: {e}"))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn handle_bg_attach(_id: &str) -> anyhow::Result<()> {
    eprintln!("Background sessions are only supported on Unix systems.");
    std::process::exit(1);
}

/// View background session logs (non-streaming).
pub fn handle_bg_logs(id: &str) -> anyhow::Result<()> {
    let content =
        archon_session::attach::view_logs(id)
            .map_err(|e| anyhow::anyhow!("failed to read logs for session {id}: {e}"))?;
    print!("{content}");
    Ok(())
}

/// Launch a background session and exit.
#[cfg(unix)]
pub fn handle_bg_launch(cli: &Cli) -> anyhow::Result<()> {
    let query = match &cli.bg {
        Some(Some(q)) => q.clone(),
        Some(None) => {
            // Read from stdin
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| anyhow::anyhow!("failed to read stdin: {e}"))?;
            buf
        }
        None => unreachable!(),
    };

    if query.trim().is_empty() {
        eprintln!("error: no query provided for background session");
        std::process::exit(1);
    }

    let archon_binary = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("failed to resolve archon binary path: {e}"))?;

    let session_id = archon_session::background::launch_background(&query, cli.bg_name.as_deref(), &archon_binary)
        .map_err(|e| anyhow::anyhow!("failed to launch background session: {e}"))?;

    let short_id = if session_id.len() > 8 { &session_id[..8] } else { &session_id };
    eprintln!("Background session started: {session_id}");
    eprintln!("  Attach: archon --attach {short_id}");
    eprintln!("  Logs:   archon --logs {short_id}");
    eprintln!("  Kill:   archon --kill {short_id}");
    eprintln!("  List:   archon --ps");
    Ok(())
}

#[cfg(not(unix))]
pub fn handle_bg_launch(_cli: &Cli) -> anyhow::Result<()> {
    eprintln!("Background sessions are only supported on Unix systems.");
    std::process::exit(1);
}
