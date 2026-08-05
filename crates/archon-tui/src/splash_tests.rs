use super::{intj_theme, logo_activity_line};

/// The visible width of a rendered line, borders included.
fn rendered(line: &ratatui::text::Line<'_>) -> String {
    line.spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<String>()
}

/// An activity description longer than its column used to run past the column,
/// overwrite the closing border, and get cut off by the frame edge mid-word.
#[test]
fn a_long_activity_description_is_cut_with_a_marker() {
    let t = intj_theme();
    let long = "just now Memory garden: 3 duplicate(s) merged, 2 fragment(s) \
                merged, 4 stale pruned, 1 pair(s) awaiting review";
    let line = logo_activity_line(&t, 80, "logo", long);
    let out = rendered(&line);

    assert_eq!(
        out.chars().count(),
        80 - 2 + 2,
        "the line must fill exactly the width it was given, borders included"
    );
    assert!(
        out.ends_with('│'),
        "the closing border must survive an over-long description, got: {out}"
    );
    assert!(
        out.contains("..."),
        "a description that was cut must say so, got: {out}"
    );
}

#[test]
fn a_short_activity_description_is_left_intact() {
    let t = intj_theme();
    let line = logo_activity_line(&t, 80, "logo", "1m ago   Empty session");
    let out = rendered(&line);

    assert!(out.contains("1m ago   Empty session"));
    assert!(!out.contains("..."), "nothing to cut, got: {out}");
}

/// A column too narrow for the marker still may not overflow. The logo is
/// passed empty here because it is deliberately never cut -- fixed art the
/// layout is sized around -- and would dominate the measurement.
#[test]
fn a_column_with_no_room_for_the_marker_still_fits() {
    let t = intj_theme();
    let line = logo_activity_line(&t, 8, "", "a very long description");
    let out = rendered(&line);
    assert_eq!(out.chars().count(), 8);
}
