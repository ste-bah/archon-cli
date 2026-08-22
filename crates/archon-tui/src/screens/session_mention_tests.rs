//! Tests for the `@`-mention picker (#200 Phase 4).

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn candidate(id: &str, label: &str) -> MentionCandidate {
    MentionCandidate {
        id: id.into(),
        label: label.into(),
        detail: "4 msgs · 1h".into(),
    }
}

/// Most-recent-first, as the source contract requires.
fn three() -> Vec<MentionCandidate> {
    vec![
        candidate("aa-newest", "refactor the parser"),
        candidate("bb-middle", "chase the flaky test"),
        candidate("cc-oldest", "parser notes"),
    ]
}

fn ids(picker: &SessionMentionPicker) -> Vec<String> {
    picker
        .list
        .items()
        .iter()
        .map(|entry| entry.id.clone())
        .collect()
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

#[test]
fn an_empty_query_offers_everything_most_recent_first() {
    let picker = SessionMentionPicker::new(three());
    assert_eq!(ids(&picker), ["aa-newest", "bb-middle", "cc-oldest"]);
}

/// The whole point of ranking by evidence: an id prefix wins even though the
/// matching session is the oldest one on the list.
#[test]
fn an_id_prefix_outranks_recency() {
    let mut picker = SessionMentionPicker::new(three());
    picker.set_query("cc");
    assert_eq!(ids(&picker), ["cc-oldest"]);
}

#[test]
fn an_id_match_outranks_a_label_match() {
    let mut picker = SessionMentionPicker::new(vec![
        candidate("zz-1", "middle parser work"),
        candidate("parser-2", "unrelated"),
    ]);
    picker.set_query("parser");
    assert_eq!(
        ids(&picker),
        ["parser-2", "zz-1"],
        "naming a session by id must beat mentioning the word in a summary"
    );
}

#[test]
fn an_earlier_label_match_outranks_a_later_one() {
    let mut picker = SessionMentionPicker::new(vec![
        candidate("aa", "chase the flaky parser test"),
        candidate("bb", "parser rewrite"),
    ]);
    picker.set_query("parser");
    assert_eq!(ids(&picker), ["bb", "aa"]);
}

/// Recency survives as the tiebreaker when the evidence is equal.
#[test]
fn equal_scores_keep_the_recency_order() {
    let mut picker = SessionMentionPicker::new(vec![
        candidate("aa", "parser one"),
        candidate("bb", "parser two"),
    ]);
    picker.set_query("parser");
    assert_eq!(ids(&picker), ["aa", "bb"]);
}

#[test]
fn non_matches_are_dropped_rather_than_demoted() {
    let mut picker = SessionMentionPicker::new(three());
    picker.set_query("flaky");
    assert_eq!(ids(&picker), ["bb-middle"]);
}

#[test]
fn matching_ignores_case() {
    let mut picker = SessionMentionPicker::new(three());
    picker.set_query("PARSER");
    // Both label matches survive the case difference. `cc-oldest` leads
    // because "parser notes" matches at position 0 and "refactor the parser"
    // does not — match position still decides, case does not.
    assert_eq!(ids(&picker), ["cc-oldest", "aa-newest"]);
}

/// Backspacing out of a narrow query has to bring the list back — the picker
/// re-filters from the full set every time, it does not narrow in place.
#[test]
fn widening_the_query_restores_the_dropped_rows() {
    let mut picker = SessionMentionPicker::new(three());
    picker.set_query("flaky");
    assert_eq!(picker.len(), 1);
    picker.set_query("");
    assert_eq!(picker.len(), 3);
}

#[test]
fn the_cursor_survives_the_list_shrinking_under_it() {
    let mut picker = SessionMentionPicker::new(three());
    picker.move_down();
    picker.move_down();
    assert_eq!(picker.selected_index(), 2);
    picker.set_query("flaky");
    assert_eq!(picker.selected_index(), 0, "must not point past the end");
    assert_eq!(picker.selected().map(|c| c.id.as_str()), Some("bb-middle"));
}

#[test]
fn the_cursor_wraps() {
    let mut picker = SessionMentionPicker::new(three());
    picker.move_up();
    assert_eq!(picker.selected_index(), 2);
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn draw(picker: &SessionMentionPicker) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(96, 24)).expect("terminal");
    terminal
        .draw(|frame| picker.render(frame, frame.area(), &crate::theme::dark_theme()))
        .expect("draw mention picker");
    terminal
}

fn text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn ids_and_summaries_are_drawn_with_a_header() {
    let rendered = text(&draw(&SessionMentionPicker::new(three())));
    assert!(rendered.contains("aa-newest"), "{rendered}");
    assert!(rendered.contains("refactor the parser"), "{rendered}");
    assert!(rendered.contains("Session"), "header missing: {rendered}");
}

#[test]
fn the_title_names_the_keys_that_work() {
    assert!(text(&draw(&SessionMentionPicker::new(three()))).contains("Enter insert"));
}

/// Three different nothings, three different sentences. Each has to reach the
/// screen — an empty bordered box is indistinguishable from a broken widget.
#[test]
fn no_sessions_at_all_is_stated_in_words() {
    let rendered = text(&draw(&SessionMentionPicker::new(Vec::new())));
    assert!(rendered.contains("No other session"), "{rendered}");
}

#[test]
fn no_match_for_the_query_is_stated_in_words() {
    let mut picker = SessionMentionPicker::new(three());
    picker.set_query("zzzz");
    let rendered = text(&draw(&picker));
    assert!(rendered.contains("No session matches"), "{rendered}");
}

#[test]
fn a_missing_source_says_so_rather_than_looking_empty() {
    let rendered = text(&draw(&SessionMentionPicker::unavailable()));
    assert!(rendered.contains("unavailable"), "{rendered}");
}

#[test]
fn the_overlay_leaves_the_input_line_visible() {
    let terminal = draw(&SessionMentionPicker::new(three()));
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    let bottom: String = (0..area.width)
        .map(|x| buffer[(x, area.height - 1)].symbol().to_string())
        .collect();
    assert!(
        bottom.trim().is_empty(),
        "the picker painted over the row the user is typing on"
    );
}
