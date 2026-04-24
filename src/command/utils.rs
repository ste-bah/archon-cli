//! Helper utilities extracted from main.rs to reduce main.rs from 1349 to < 500 lines.

// ---------------------------------------------------------------------------
// Date/time helpers
// ---------------------------------------------------------------------------

/// Parse a date string as either RFC 3339 or YYYY-MM-DD (assumes midnight UTC).
pub fn parse_datetime(s: &str) -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
    // Try RFC 3339 first.
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&chrono::Utc));
    }
    // Try YYYY-MM-DD.
    if let Ok(nd) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let naive = nd
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow::anyhow!("invalid date: {s}"))?;
        return Ok(naive.and_utc());
    }
    Err(anyhow::anyhow!(
        "invalid date format: {s} (expected RFC 3339 or YYYY-MM-DD)"
    ))
}

/// List recent sessions for `--resume` with no ID.
pub async fn handle_resume_list() -> anyhow::Result<()> {
    let db_path = archon_session::storage::default_db_path();
    let store = archon_session::storage::SessionStore::open(&db_path)
        .map_err(|e| anyhow::anyhow!("failed to open session database: {e}"))?;

    let sessions = store
        .list_sessions(20)
        .map_err(|e| anyhow::anyhow!("failed to list sessions: {e}"))?;

    if sessions.is_empty() {
        eprintln!("No previous sessions found.");
    } else {
        eprintln!("Recent sessions:");
        for session in &sessions {
            eprintln!("  {}", archon_session::resume::format_session_line(session));
        }
        eprintln!("\nUse: archon --resume <session-id>");
    }
    Ok(())
}

/// Load resume messages for `--resume <id>`.
pub fn load_resume_messages(session_id: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let db_path = archon_session::storage::default_db_path();
    let store = archon_session::storage::SessionStore::open(&db_path)
        .map_err(|e| anyhow::anyhow!("failed to open session database: {e}"))?;
    let (meta, raw_messages) = archon_session::resume::resume_session(&store, session_id)
        .map_err(|e| anyhow::anyhow!("failed to resume session: {e}"))?;
    eprintln!(
        "Resumed session {} ({} messages, {} tokens)",
        &meta.id[..8.min(meta.id.len())],
        meta.message_count,
        meta.total_tokens,
    );
    // Parse stored JSON strings back into Values
    let messages: Vec<serde_json::Value> = raw_messages
        .iter()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect();
    Ok(messages)
}

// ---------------------------------------------------------------------------
// Tool filtering
// ---------------------------------------------------------------------------

/// Apply `--tools` (whitelist) and `--disallowed-tools` (blacklist) from
/// resolved CLI flags to the tool registry.
pub fn apply_tool_filters(
    registry: &mut archon_core::dispatch::ToolRegistry,
    flags: &archon_core::cli_flags::ResolvedFlags,
) {
    if let Some(ref whitelist) = flags.tool_whitelist {
        let names: Vec<&str> = whitelist.iter().map(|s| s.as_str()).collect();
        registry.filter_whitelist(&names);
        tracing::info!("tool whitelist applied: {} tools retained", names.len());
    }
    if let Some(ref blacklist) = flags.tool_blacklist {
        let names: Vec<&str> = blacklist.iter().map(|s| s.as_str()).collect();
        registry.filter_blacklist(&names);
        tracing::info!("tool blacklist applied: removed {} patterns", names.len());
    }
}

// ---------------------------------------------------------------------------
// String utilities
// ---------------------------------------------------------------------------

/// Truncate a string to at most `max` bytes, appending "..." if truncated.
/// Safe for multi-byte UTF-8: always splits on a char boundary.
pub fn truncate_str(s: &str, max: usize) -> String {
    let trimmed = s.replace('\n', " ");
    if trimmed.len() <= max {
        trimmed
    } else {
        let mut end = max.saturating_sub(3);
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &trimmed[..end])
    }
}

// ---------------------------------------------------------------------------
// Account utilities
// ---------------------------------------------------------------------------

/// Fetch account UUID from Anthropic OAuth profile endpoint.
pub async fn fetch_account_uuid(auth: &archon_llm::auth::AuthProvider) -> String {
    let (header_name, header_value) = auth.header();

    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let result = client
        .get("https://api.anthropic.com/api/oauth/profile")
        .header(&header_name, &header_value)
        .send()
        .await;

    match result {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.text().await
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&body)
            {
                // Profile response: { "account": { "uuid": "..." }, "organization": { ... } }
                if let Some(uuid) = json
                    .get("account")
                    .and_then(|a| a.get("uuid"))
                    .and_then(|v| v.as_str())
                {
                    tracing::info!("fetched account_uuid: {}", &uuid[..8.min(uuid.len())]);
                    return uuid.to_string();
                }
            }
            tracing::warn!("profile response missing account_uuid");
            String::new()
        }
        Ok(resp) => {
            tracing::warn!("profile fetch failed: HTTP {}", resp.status());
            String::new()
        }
        Err(e) => {
            tracing::warn!("profile fetch error: {e}");
            String::new()
        }
    }
}
