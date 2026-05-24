use anyhow::Result;

use crate::cli_args::Cli;

pub(crate) async fn handle_resume_list_if_requested(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
) -> Result<bool> {
    if let Some(None) = &cli.resume {
        crate::session::handle_resume_list_with_config(config).await?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn load_explicit_resume_messages(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
) -> Result<Option<Vec<serde_json::Value>>> {
    if let Some(Some(ref id)) = cli.resume {
        return Ok(Some(crate::session::load_resume_messages_with_config(
            id, config,
        )?));
    }
    Ok(None)
}

pub(crate) fn maybe_continue_session(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
    resume_messages: &mut Option<Vec<serde_json::Value>>,
) {
    if !cli.continue_session || resume_messages.is_some() {
        return;
    }
    match load_most_recent_session_messages(config) {
        Ok(Some((id, messages))) => {
            eprintln!("Continuing session {} ...", &id[..8.min(id.len())]);
            *resume_messages = Some(messages);
        }
        Ok(None) => eprintln!("No previous session found in this directory."),
        Err(error) => eprintln!("{error}"),
    }
}

pub(crate) fn maybe_auto_resume(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
    resume_messages: &mut Option<Vec<serde_json::Value>>,
) {
    if cli.resume.is_some() || cli.continue_session {
        tracing::info!("auto_resume: skipped (--resume specified)");
    } else if cli.no_resume {
        tracing::info!("auto_resume: skipped (--no-resume)");
    } else if !config.session.auto_resume {
        tracing::info!("auto_resume: skipped (session.auto_resume=false)");
    } else if resume_messages.is_none() {
        apply_auto_resume(config, resume_messages);
    }
}

fn apply_auto_resume(
    config: &archon_core::config::ArchonConfig,
    resume_messages: &mut Option<Vec<serde_json::Value>>,
) {
    match load_most_recent_session_messages(config) {
        Ok(Some((id, messages))) => {
            tracing::info!(
                "auto_resume: found prior session {}",
                &id[..8.min(id.len())]
            );
            eprintln!(
                "Auto-resumed session {} — pass --no-resume to start fresh.",
                &id[..8.min(id.len())],
            );
            *resume_messages = Some(messages);
        }
        Ok(None) => tracing::info!("auto_resume: no prior session for this directory"),
        Err(error) => tracing::warn!("auto_resume: {error}"),
    }
}

fn load_most_recent_session_messages(
    config: &archon_core::config::ArchonConfig,
) -> Result<Option<(String, Vec<serde_json::Value>)>> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let cwd_str = cwd.to_string_lossy().to_string();
    let db_path = crate::command::store_paths::session_db_path(config);
    let store = archon_session::storage::SessionStore::open(&db_path)
        .map_err(|error| anyhow::anyhow!("Session store open failed: {error}"))?;
    let Some(meta) = archon_session::listing::most_recent_in_directory(&store, &cwd_str)
        .map_err(|error| anyhow::anyhow!("Session lookup failed: {error}"))?
    else {
        return Ok(None);
    };
    let (_metadata, raw_messages) = archon_session::resume::resume_session(&store, &meta.id)
        .map_err(|error| anyhow::anyhow!("Failed to continue session: {error}"))?;
    let messages = raw_messages
        .iter()
        .filter_map(|message| serde_json::from_str(message).ok())
        .collect();
    Ok(Some((meta.id, messages)))
}
