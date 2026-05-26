use archon_tui::app::{App, EvidenceRowPayload, EvidenceViewState, ViewId};

#[test]
fn open_view_sets_cognitive_evidence_overlay_source_of_truth() {
    let mut app = App::new();
    app.open_view(ViewId::Cognitive);

    let view = app.evidence_view.as_ref().expect("view opened");
    assert_eq!(view.view_id(), ViewId::Cognitive);
    assert!(matches!(view, EvidenceViewState::Cognitive(_)));
}

#[test]
fn open_view_with_rows_sets_cognitive_rows_from_source_of_truth() {
    let mut app = App::new();
    app.open_view_with_rows(
        ViewId::Cognitive,
        vec![EvidenceRowPayload {
            id: "decision-1".into(),
            title: "decision".into(),
            status: "selected".into(),
            detail: "run_tests".into(),
        }],
    );

    let view = app.evidence_view.as_ref().expect("view opened");
    let EvidenceViewState::Cognitive(screen) = view else {
        panic!("expected cognitive view");
    };
    assert_eq!(screen.len(), 1);
    assert_eq!(screen.selected().unwrap().id, "decision-1");
}
