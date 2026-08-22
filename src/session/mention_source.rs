//! Candidate rows for the `@`-mention picker (#200 Phase 4).
//!
//! `archon-tui` defines the trait and owns the overlay but has no
//! `SessionStore`, the same wall `task_overlay_store` hit. This crate has
//! both, so the two halves meet here.
//!
//! # What is offered, and what is deliberately not
//!
//! Two classes of session are filtered out before the picker ever sees them,
//! because `archon_core::session_reference::prepare_session_reference` refuses
//! both and offering a row that cannot resolve is offering a failure:
//!
//! - the session the user is in (`SelfReference`);
//! - a session with no stored messages (`Empty`).
//!
//! The filtering is here rather than in the picker because this is where the
//! same store that will later be asked to resolve the id can be consulted;
//! the picker has only the rows it is handed.

use std::sync::Arc;

use archon_session::search::{SessionSearchQuery, search_sessions};
use archon_session::storage::{SessionMetadata, SessionStore};
use archon_tui::{MentionCandidate, SessionMentionSource};

/// How many sessions the picker may offer.
///
/// Generous, because the picker filters and ranks the whole set as the user
/// types and a cut-off list would silently hide the session being searched
/// for. Small enough that the read stays cheap on every `@`.
const MAX_CANDIDATES: usize = 200;

/// Reads referenceable sessions out of the real store.
pub struct StoreMentionSource {
    store: Arc<SessionStore>,
    /// Excluded from every listing: a session cannot reference itself.
    current_session_id: String,
}

impl StoreMentionSource {
    /// Handle to inject into `AppConfig::session_mentions`.
    pub fn shared(
        store: Arc<SessionStore>,
        current_session_id: &str,
    ) -> Arc<dyn SessionMentionSource> {
        Arc::new(Self {
            store,
            current_session_id: current_session_id.to_string(),
        })
    }
}

impl SessionMentionSource for StoreMentionSource {
    fn candidates(&self) -> Vec<MentionCandidate> {
        let query = SessionSearchQuery {
            limit: MAX_CANDIDATES,
            ..SessionSearchQuery::default()
        };
        // `SessionSearchQuery::default()` sorts by date descending, which is
        // the ordering `SessionMentionSource` promises and the picker's
        // tiebreaker depends on. A failed read yields no rows; the picker then
        // says there is nothing to reference rather than pretending otherwise.
        let sessions = match search_sessions(self.store.as_ref(), &query) {
            Ok(sessions) => sessions,
            Err(error) => {
                tracing::warn!(%error, "could not list sessions for the @-mention picker");
                Vec::new()
            }
        };
        sessions
            .into_iter()
            .filter(|session| session.id != self.current_session_id)
            .filter(|session| session.message_count > 0)
            .map(|session| MentionCandidate {
                label: label_for(&session),
                detail: detail_for(&session),
                id: session.id,
            })
            .collect()
    }
}

/// What the session is about, in one column.
///
/// The name if it was given one, otherwise where it was working and on what
/// branch — which is how an unnamed session is actually recognised.
fn label_for(session: &SessionMetadata) -> String {
    if let Some(name) = session.name.as_ref().filter(|name| !name.trim().is_empty()) {
        return name.trim().to_string();
    }
    let where_ = tail_of_path(&session.working_directory);
    match session.git_branch.as_deref().filter(|b| !b.is_empty()) {
        Some(branch) => format!("{where_} ({branch})"),
        None => where_,
    }
}

/// Size and age, for telling two sessions in the same directory apart.
fn detail_for(session: &SessionMetadata) -> String {
    let count = session.message_count;
    let unit = if count == 1 { "msg" } else { "msgs" };
    format!("{count} {unit} · {}", age_of(&session.last_active))
}

/// The last two components of a path, which is usually the recognisable part.
fn tail_of_path(path: &str) -> String {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    let tail = parts.len().saturating_sub(2);
    let joined = parts[tail..].join("/");
    if joined.is_empty() {
        "(unknown directory)".to_string()
    } else {
        joined
    }
}

/// How long ago, coarsely.
///
/// An unparseable timestamp is shown as `?` rather than as "now": a stored
/// value this code cannot read is not evidence the session is fresh, and
/// guessing would put an ancient session at the top of the user's attention.
fn age_of(last_active: &str) -> String {
    let Ok(when) = chrono::DateTime::parse_from_rfc3339(last_active) else {
        return "?".to_string();
    };
    let elapsed = chrono::Utc::now().signed_duration_since(when.with_timezone(&chrono::Utc));
    let minutes = elapsed.num_minutes();
    if minutes < 1 {
        return "just now".to_string();
    }
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = elapsed.num_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", elapsed.num_days())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(id: &str, count: u64) -> SessionMetadata {
        SessionMetadata {
            id: id.to_string(),
            created_at: String::new(),
            last_active: String::new(),
            working_directory: "/home/steve/code/archon-cli".to_string(),
            git_branch: Some("main".to_string()),
            model: "test".to_string(),
            message_count: count,
            total_tokens: 0,
            total_cost: 0.0,
            schema_version: 1,
            name: None,
            parent_session_id: None,
        }
    }

    #[test]
    fn an_unnamed_session_is_labelled_by_where_it_was_working() {
        assert_eq!(label_for(&metadata("a", 3)), "code/archon-cli (main)");
    }

    #[test]
    fn a_named_session_uses_its_name() {
        let mut session = metadata("a", 3);
        session.name = Some("  parser rewrite  ".to_string());
        assert_eq!(label_for(&session), "parser rewrite");
    }

    #[test]
    fn the_message_count_is_pluralised() {
        assert!(detail_for(&metadata("a", 1)).starts_with("1 msg ·"));
        assert!(detail_for(&metadata("a", 4)).starts_with("4 msgs ·"));
    }

    /// An unreadable timestamp must not read as "just now".
    #[test]
    fn an_unparseable_timestamp_is_admitted_rather_than_guessed() {
        assert_eq!(age_of("not a date"), "?");
        assert_eq!(age_of(""), "?");
    }

    #[test]
    fn a_recent_timestamp_reads_as_recent() {
        let when = chrono::Utc::now() - chrono::Duration::minutes(5);
        assert_eq!(age_of(&when.to_rfc3339()), "5m ago");
    }

    #[test]
    fn a_bare_directory_still_produces_a_label() {
        let mut session = metadata("a", 1);
        session.working_directory = "/".to_string();
        session.git_branch = None;
        assert_eq!(label_for(&session), "(unknown directory)");
    }
}
