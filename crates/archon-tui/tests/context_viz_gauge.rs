use archon_tui::context_viz::ContextViz;

#[test]
fn ninety_percent_fill_triggers_warn() {
    let mut viz = ContextViz::new(100_000);
    viz.update(90_000, 5_000, 100_000);
    assert!(viz.is_warn());
}

#[test]
fn below_ninety_no_warn() {
    let mut viz = ContextViz::new(100_000);
    viz.update(80_000, 5_000, 100_000);
    assert!(!viz.is_warn());
}

#[test]
fn history_tracks_updates() {
    let mut viz = ContextViz::new(100_000);
    viz.update(10_000, 0, 100_000);
    viz.update(20_000, 0, 100_000);
    viz.update(30_000, 0, 100_000);
    assert_eq!(viz.history_len(), 3);
}
