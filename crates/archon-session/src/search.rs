//! Session search and statistics.

use chrono::{DateTime, Utc};

use crate::storage::{SessionError, SessionMetadata, SessionStore};

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

/// Which field to sort sessions by.
#[derive(Debug, Clone, Copy)]
pub enum SortField {
    Date,
    Duration,
    Tokens,
    Messages,
}

/// Sort direction.
#[derive(Debug, Clone, Copy)]
pub enum SortOrder {
    Asc,
    Desc,
}

/// A structured query for filtering, sorting, and limiting session results.
#[derive(Debug, Clone)]
pub struct SessionSearchQuery {
    pub branch: Option<String>,
    pub directory: Option<String>,
    pub after: Option<DateTime<Utc>>,
    pub before: Option<DateTime<Utc>>,
    /// Full-text search in session messages.
    pub text: Option<String>,
    pub tag: Option<String>,
    pub sort_by: SortField,
    pub sort_order: SortOrder,
    pub limit: usize,
}

impl Default for SessionSearchQuery {
    fn default() -> Self {
        Self {
            branch: None,
            directory: None,
            after: None,
            before: None,
            text: None,
            tag: None,
            sort_by: SortField::Date,
            sort_order: SortOrder::Desc,
            limit: 20,
        }
    }
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

/// Aggregate statistics across all sessions.
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub total_sessions: usize,
    pub total_tokens: u64,
    pub total_messages: u64,
    pub avg_duration_secs: f64,
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Search sessions in the store according to the provided query.
///
/// Filtering is done in-memory after loading all sessions. For typical usage
/// (hundreds to low-thousands of sessions) this is perfectly efficient.
pub fn search_sessions(
    store: &SessionStore,
    query: &SessionSearchQuery,
) -> Result<Vec<SessionMetadata>, SessionError> {
    // Load a generous set from the store (we filter in Rust).
    let all = store.list_sessions(u32::MAX)?;

    let mut results: Vec<SessionMetadata> = all
        .into_iter()
        .filter(|s| filter_branch(s, &query.branch))
        .filter(|s| filter_directory(s, &query.directory))
        .filter(|s| filter_after(s, &query.after))
        .filter(|s| filter_before(s, &query.before))
        .collect();

    // Full-text search on messages (expensive — done after metadata filters).
    if let Some(ref text) = query.text {
        let needle = text.to_lowercase();
        results.retain(|s| {
            match store.load_messages(&s.id) {
                Ok(msgs) => msgs
                    .iter()
                    .any(|m| m.to_lowercase().contains(&needle)),
                Err(_) => false,
            }
        });
    }

    // Tag filter.
    if let Some(ref tag) = query.tag {
        let tag_lower = tag.to_lowercase();
        results.retain(|s| {
            match crate::metadata::get_tags(store, &s.id) {
                Ok(tags) => tags.iter().any(|t| t.to_lowercase() == tag_lower),
                Err(_) => false,
            }
        });
    }

    // Sort.
    sort_sessions(&mut results, query.sort_by, query.sort_order);

    // Limit.
    results.truncate(query.limit);

    Ok(results)
}

/// Compute aggregate statistics for all sessions in the store.
pub fn session_stats(store: &SessionStore) -> Result<SessionStats, SessionError> {
    let all = store.list_sessions(u32::MAX)?;

    let total_sessions = all.len();
    let total_tokens: u64 = all.iter().map(|s| s.total_tokens).sum();
    let total_messages: u64 = all.iter().map(|s| s.message_count).sum();

    let total_duration_secs: f64 = all
        .iter()
        .filter_map(|s| {
            let created = chrono::DateTime::parse_from_rfc3339(&s.created_at).ok()?;
            let last = chrono::DateTime::parse_from_rfc3339(&s.last_active).ok()?;
            let dur = last.signed_duration_since(created);
            Some(dur.num_seconds().max(0) as f64)
        })
        .sum();

    let avg_duration_secs = if total_sessions > 0 {
        total_duration_secs / total_sessions as f64
    } else {
        0.0
    };

    Ok(SessionStats {
        total_sessions,
        total_tokens,
        total_messages,
        avg_duration_secs,
    })
}

// ---------------------------------------------------------------------------
// Filter helpers
// ---------------------------------------------------------------------------

fn filter_branch(s: &SessionMetadata, branch: &Option<String>) -> bool {
    match branch {
        Some(b) => s.git_branch.as_deref() == Some(b.as_str()),
        None => true,
    }
}

fn filter_directory(s: &SessionMetadata, dir: &Option<String>) -> bool {
    match dir {
        Some(d) => s.working_directory == *d,
        None => true,
    }
}

fn filter_after(s: &SessionMetadata, after: &Option<DateTime<Utc>>) -> bool {
    match after {
        Some(cutoff) => {
            chrono::DateTime::parse_from_rfc3339(&s.created_at)
                .map(|dt| dt >= *cutoff)
                .unwrap_or(false)
        }
        None => true,
    }
}

fn filter_before(s: &SessionMetadata, before: &Option<DateTime<Utc>>) -> bool {
    match before {
        Some(cutoff) => {
            chrono::DateTime::parse_from_rfc3339(&s.created_at)
                .map(|dt| dt <= *cutoff)
                .unwrap_or(false)
        }
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Sort helpers
// ---------------------------------------------------------------------------

fn sort_sessions(sessions: &mut [SessionMetadata], field: SortField, order: SortOrder) {
    sessions.sort_by(|a, b| {
        let cmp = match field {
            SortField::Date => a.last_active.cmp(&b.last_active),
            SortField::Duration => {
                duration_secs(a).partial_cmp(&duration_secs(b)).unwrap_or(std::cmp::Ordering::Equal)
            }
            SortField::Tokens => a.total_tokens.cmp(&b.total_tokens),
            SortField::Messages => a.message_count.cmp(&b.message_count),
        };
        match order {
            SortOrder::Asc => cmp,
            SortOrder::Desc => cmp.reverse(),
        }
    });
}

fn duration_secs(s: &SessionMetadata) -> f64 {
    let created = chrono::DateTime::parse_from_rfc3339(&s.created_at);
    let last = chrono::DateTime::parse_from_rfc3339(&s.last_active);
    match (created, last) {
        (Ok(c), Ok(l)) => l.signed_duration_since(c).num_seconds().max(0) as f64,
        _ => 0.0,
    }
}
