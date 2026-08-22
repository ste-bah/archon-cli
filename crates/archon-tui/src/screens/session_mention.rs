//! The `@`-mention picker for cross-session references (#200 Phase 4).
//!
//! Layer 1 module — no imports from screens/ or app/.
//!
//! The grammar that decides when this opens lives in
//! [`archon_core::mention`], shared with the send-time resolver so the two
//! cannot disagree about what a mention is. This module is only the list: what
//! to offer, in what order, and how to draw it.
//!
//! # Not the `/fork-at` shape
//!
//! The branch picker resolves by calling [`crate::app::App`]'s `set_text` with
//! a whole slash command, discarding whatever was in the buffer. That is right
//! for `/fork-at`, which *is* the command. A mention is a noun inside a
//! sentence the user is still composing, so this one resolves **in place**:
//! [`archon_core::mention::replace_active`] swaps the typed mention for
//! `@session:<id>` and leaves the rest of the line, and the caret, alone.
//!
//! # Where the rows come from
//!
//! This crate cannot reach a `SessionStore` — the same wall the tasks overlay
//! hit — so candidates arrive through [`SessionMentionSource`], injected from
//! the bin crate. `None` there is not silently an empty list: the overlay says
//! the surface is unavailable, because "no other sessions exist" and "nothing
//! wired the list up" are different answers and only one of them is the user's
//! problem.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::widgets::Row;

use crate::theme::Theme;
use crate::virtual_list::VirtualList;

/// One session that could be referenced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionCandidate {
    /// The session id, which is what `@session:<id>` carries.
    pub id: String,
    /// The session's name, or a summary of how it opened.
    pub label: String,
    /// Size and age, for telling two similar sessions apart.
    pub detail: String,
}

/// Supplies referenceable sessions, most recently active first.
///
/// Ordering is part of the contract rather than a courtesy: the picker uses
/// position in this list as its recency prior (see [`SessionMentionPicker::set_query`]),
/// which keeps date handling in the one place that has the real timestamps.
///
/// Implementations are expected to exclude the current session and any session
/// with no stored messages. Both are guaranteed errors from
/// `archon_core::session_reference::prepare_session_reference`, and offering a
/// row that cannot resolve is offering a failure.
pub trait SessionMentionSource: Send + Sync {
    fn candidates(&self) -> Vec<MentionCandidate>;
}

/// Picker over the sessions an `@` could resolve to.
#[derive(Debug)]
pub struct SessionMentionPicker {
    all: Vec<MentionCandidate>,
    list: VirtualList<MentionCandidate>,
    query: String,
    available: bool,
}

impl SessionMentionPicker {
    /// Open over `candidates`, most-recent-first.
    pub fn new(candidates: Vec<MentionCandidate>) -> Self {
        let mut picker = Self {
            all: candidates,
            list: VirtualList::new(Vec::new(), 10),
            query: String::new(),
            available: true,
        };
        picker.apply();
        picker
    }

    /// Open in the state that says no source was injected.
    pub fn unavailable() -> Self {
        Self {
            all: Vec::new(),
            list: VirtualList::new(Vec::new(), 10),
            query: String::new(),
            available: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    crate::virtual_list::delegate_virtual_list!(list, MentionCandidate);

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Re-filter and re-rank for what has been typed after the `@`.
    ///
    /// # Ranking
    ///
    /// Score first, recency second, and never the other way round:
    ///
    /// 1. an id that *starts with* the query — the user is typing an id they
    ///    already know, and nothing should outrank that;
    /// 2. an id that contains it — ids are often prefixed, so the memorable
    ///    part is frequently in the middle;
    /// 3. a label match, better the earlier it appears;
    /// 4. anything else is dropped, not demoted. A picker that keeps
    ///    non-matches on screen makes the user re-read the whole list to find
    ///    out their query matched nothing.
    ///
    /// Recency only breaks ties. A text match is *evidence* about which
    /// session is meant; recency is a prior. Blending the two into one weight
    /// would let a session that happens to have been touched recently outrank
    /// one the user literally named, and would make the top row move around as
    /// unrelated sessions get used. With an empty query — the instant `@` is
    /// typed — there is no evidence at all, so the order is pure recency,
    /// which is the best guess available and matches what the user was most
    /// likely doing five minutes ago.
    ///
    /// The sort is stable and the source hands rows over most-recent-first, so
    /// "recency breaks ties" needs no timestamp here.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.apply();
    }

    fn apply(&mut self) {
        let needle = self.query.to_lowercase();
        let mut scored: Vec<(u32, MentionCandidate)> = self
            .all
            .iter()
            .filter_map(|candidate| score(candidate, &needle).map(|rank| (rank, candidate.clone())))
            .collect();
        scored.sort_by(|left, right| right.0.cmp(&left.0));
        self.list
            .set_items(scored.into_iter().map(|(_, entry)| entry).collect());
    }

    /// Draw the candidates into a centred rect inside `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const TITLE: &str = " Reference a session — Up/Down select · Enter insert · Esc cancel ";

        if let Some(reason) = self.nothing_to_show() {
            crate::overlay::message(f, area, TITLE, reason, theme);
            return;
        }

        let (region, block) =
            crate::overlay::open(f, area, self.list.len() as u16 + 3, TITLE, theme);
        let rows: Vec<Row> = self
            .list
            .items()
            .iter()
            .map(|entry| {
                Row::new([entry.id.clone(), entry.label.clone(), entry.detail.clone()])
                    .style(crate::overlay::body_style(theme))
            })
            .collect();
        crate::overlay::render_table(
            f,
            region,
            block,
            Row::new(["Session", "About", "Activity"]),
            rows,
            &[
                Constraint::Length(20),
                Constraint::Min(20),
                Constraint::Length(18),
            ],
            self.list.selected_index(),
            theme,
        );
    }

    /// Why the list is empty, in words, or `None` when it is not.
    ///
    /// Three different nothings, and conflating them is how a user concludes
    /// the feature is broken when it is working.
    fn nothing_to_show(&self) -> Option<&'static str> {
        if !self.available {
            return Some(
                "Session references are unavailable: no session source was provided to the TUI.",
            );
        }
        if self.all.is_empty() {
            return Some(
                "No other session has stored messages to reference yet. \
                 Esc to carry on typing.",
            );
        }
        if self.list.is_empty() {
            return Some("No session matches what you have typed. Backspace to widen the search.");
        }
        None
    }
}

/// Rank one candidate against a lowercased query, or `None` to drop it.
fn score(candidate: &MentionCandidate, needle: &str) -> Option<u32> {
    if needle.is_empty() {
        return Some(0);
    }
    let id = candidate.id.to_lowercase();
    if id.starts_with(needle) {
        return Some(3000);
    }
    if id.contains(needle) {
        return Some(2000);
    }
    // Earlier matches score higher, floored so a very long label cannot push a
    // genuine match below the id tiers.
    candidate
        .label
        .to_lowercase()
        .find(needle)
        .map(|at| 1000_u32.saturating_sub(u32::try_from(at).unwrap_or(u32::MAX).min(900)))
}

#[cfg(test)]
#[path = "session_mention_tests.rs"]
mod tests;
