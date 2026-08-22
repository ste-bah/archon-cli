//! Session search the model can call (#189 Phase 2).
//!
//! The search itself has existed and been tested for a long time
//! (`archon_session::search::search_sessions`), reachable only through the
//! `/sessions` slash command — that is, only when a human typed it. #187 made
//! skills model-invocable but deliberately gates the catalogue on
//! `Skill::agent_invocable`, and descriptor-only skills returning
//! `SkillOutput::Text` render in the TUI and never reach the model.
//! `SessionsSkill` is exactly that, so #187 formalised the boundary that kept
//! session search human-only rather than crossing it.
//!
//! A tool is the right crossing. The skill route would have made one code path
//! serve both a human reading a terminal table and a model parsing results,
//! which is the pairing #187 separated on purpose.
//!
//! What comes back here is structured — id, name, when, size, and the line that
//! matched — not the `/sessions` listing, which is laid out for someone reading
//! a terminal.

use std::path::PathBuf;

use serde_json::json;

use crate::tool::{
    PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult, WorkingTreeEffect,
};

/// Results returned when the model does not ask for a specific number.
const DEFAULT_LIMIT: usize = 10;
/// Hard ceiling regardless of what was asked for.
const MAX_LIMIT: usize = 50;
/// Longest excerpt kept per session.
const MAX_EXCERPT_CHARS: usize = 240;
/// Ceiling on the whole payload.
///
/// A broad query can match hundreds of sessions across months of work, and a
/// tool added to relieve context pressure must not become a way to cause it.
const MAX_RESPONSE_BYTES: usize = 16_000;

/// Searches stored sessions and returns structured matches.
#[derive(Default)]
pub struct SessionSearchTool {
    /// Path from configuration, when one was set.
    ///
    /// Resolution order matches `session_db_path` in the bin crate exactly:
    /// the environment override wins, then this, then the default location.
    /// Injected because this crate cannot see `ArchonConfig`, and guessing
    /// would mean silently searching a different database than `/sessions`
    /// reads — the two must never disagree about which store they mean.
    configured_db_path: Option<PathBuf>,
}

impl SessionSearchTool {
    #[must_use]
    pub fn new(configured_db_path: Option<PathBuf>) -> Self {
        Self { configured_db_path }
    }

    fn db_path(&self) -> PathBuf {
        std::env::var_os("ARCHON_SESSION_DB_PATH")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| self.configured_db_path.clone())
            .unwrap_or_else(archon_session::storage::default_db_path)
    }
}

#[async_trait::async_trait]
impl Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "SessionSearch"
    }

    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }

    fn description(&self) -> &str {
        "Search past sessions by text, working directory, git branch or date. \
         Use when the user refers to earlier work — \"what did we decide about X\", \
         \"the session where we fixed Y\" — or when you need context from a \
         conversation that is not in this one. Returns session ids, names, \
         timestamps and the matching excerpt."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Text to find in session messages (case-insensitive)"
                },
                "limit": {
                    "type": "integer",
                    "description": format!("Maximum results (default {DEFAULT_LIMIT}, max {MAX_LIMIT})")
                },
                "directory": {
                    "type": "string",
                    "description": "Only sessions whose working directory contains this"
                },
                "branch": {
                    "type": "string",
                    "description": "Only sessions on this git branch"
                },
                "after": {
                    "type": "string",
                    "description": "Only sessions active after this RFC3339 timestamp"
                },
                "before": {
                    "type": "string",
                    "description": "Only sessions active before this RFC3339 timestamp"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let text = string_arg(&input, "query");
        let query = archon_session::search::SessionSearchQuery {
            branch: string_arg(&input, "branch"),
            directory: string_arg(&input, "directory"),
            after: match timestamp_arg(&input, "after") {
                Ok(value) => value,
                Err(error) => return ToolResult::error(error),
            },
            before: match timestamp_arg(&input, "before") {
                Ok(value) => value,
                Err(error) => return ToolResult::error(error),
            },
            text: text.clone(),
            tag: None,
            limit: requested_limit(&input),
            ..Default::default()
        };

        let path = self.db_path();
        let store = match archon_session::storage::SessionStore::open(&path) {
            Ok(store) => store,
            Err(error) => {
                return ToolResult::error(format!(
                    "could not open the session database at {}: {error}",
                    path.display()
                ));
            }
        };
        let matches = match archon_session::search::search_sessions(&store, &query) {
            Ok(matches) => matches,
            Err(error) => return ToolResult::error(format!("session search failed: {error}")),
        };
        if matches.is_empty() {
            return ToolResult::success("No matching sessions.");
        }

        let rows: Vec<serde_json::Value> = matches
            .iter()
            .map(|session| {
                let mut row = json!({
                    "session_id": session.id,
                    "last_active": session.last_active,
                    "working_directory": session.working_directory,
                    "message_count": session.message_count,
                });
                if let Some(name) = &session.name {
                    row["name"] = json!(name);
                }
                if let Some(branch) = &session.git_branch {
                    row["git_branch"] = json!(branch);
                }
                if let Some(needle) = &text
                    && let Some(excerpt) = first_match(&store, &session.id, needle)
                {
                    row["excerpt"] = json!(excerpt);
                }
                row
            })
            .collect();

        render(&rows)
    }

    fn working_tree_effect(&self) -> WorkingTreeEffect {
        WorkingTreeEffect::None
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        // A read over the user's own local session store, which changes
        // nothing and leaves the machine.
        PermissionLevel::Safe
    }
}

/// Serialise, dropping trailing rows until the payload fits.
///
/// Truncating the JSON text would produce something the model cannot parse, so
/// whole rows go instead, and the response says how many.
fn render(rows: &[serde_json::Value]) -> ToolResult {
    let mut kept = rows.len();
    loop {
        let omitted = rows.len() - kept;
        let mut payload = json!({ "matches": &rows[..kept] });
        if omitted > 0 {
            payload["omitted"] = json!(omitted);
            payload["note"] = json!(
                "Results were dropped to bound the response. Narrow the query or lower `limit`."
            );
        }
        let text = serde_json::to_string_pretty(&payload)
            .unwrap_or_else(|_| "{\"matches\":[]}".to_string());
        if text.len() <= MAX_RESPONSE_BYTES || kept <= 1 {
            return ToolResult::success(text);
        }
        kept /= 2;
    }
}

fn string_arg(input: &serde_json::Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Parse an RFC3339 bound, reporting a bad one rather than ignoring it.
///
/// Silently dropping an unparseable date would answer a different question
/// than the one asked and look like a legitimate empty result.
fn timestamp_arg(
    input: &serde_json::Value,
    key: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, String> {
    let Some(raw) = string_arg(input, key) else {
        return Ok(None);
    };
    chrono::DateTime::parse_from_rfc3339(&raw)
        .map(|parsed| Some(parsed.with_timezone(&chrono::Utc)))
        .map_err(|error| format!("`{key}` must be an RFC3339 timestamp: {error}"))
}

fn requested_limit(input: &serde_json::Value) -> usize {
    input
        .get("limit")
        .and_then(|value| value.as_u64())
        .map(|value| (value as usize).clamp(1, MAX_LIMIT))
        .unwrap_or(DEFAULT_LIMIT)
}

/// The first message line containing the needle, bounded.
fn first_match(
    store: &archon_session::storage::SessionStore,
    session_id: &str,
    needle: &str,
) -> Option<String> {
    let lowered = needle.to_lowercase();
    let messages = store.load_messages(session_id).ok()?;
    let hit = messages
        .iter()
        .find(|message| message.to_lowercase().contains(&lowered))?;
    Some(truncate(hit, MAX_EXCERPT_CHARS))
}

/// Cut to at most `limit` characters, never mid-character.
fn truncate(text: &str, limit: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= limit {
        return collapsed;
    }
    let kept: String = collapsed.chars().take(limit).collect();
    format!("{kept}…")
}

#[cfg(test)]
#[path = "session_search_tests.rs"]
mod tests;
