use archon_tui::screens::docs::{DocsRow, DocsScreen, DocsStore, DocsView};
use archon_tui::screens::gametheory::{
    GameTheoryRow, GameTheoryScreen, GameTheoryStore, GameTheoryView,
};
use archon_tui::screens::learning::{LearningRow, LearningScreen, LearningStore, LearningView};
use archon_tui::screens::video::{
    FrameGroupItem, TranscriptSegmentItem, VideoScreen, VideoSourceItem,
};
use archon_tui::screens::workflow::{WorkflowRow, WorkflowScreen, WorkflowStore, WorkflowView};
use archon_tui::theme::intj_theme;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

struct FixtureStore;

impl GameTheoryStore for FixtureStore {
    fn load_rows(&self, view: &GameTheoryView) -> Vec<GameTheoryRow> {
        match view {
            GameTheoryView::Main => vec![GameTheoryRow {
                id: "gt-run".into(),
                label: "Run".into(),
                status: "completed".into(),
                detail: "84 specialists".into(),
            }],
            GameTheoryView::RunDetail { run_id } => vec![GameTheoryRow {
                id: run_id.clone(),
                label: "nash-equilibrium-finder".into(),
                status: "completed".into(),
                detail: "artifact persisted".into(),
            }],
            GameTheoryView::Specimens => vec![GameTheoryRow {
                id: "specimen".into(),
                label: "Prisoner's dilemma".into(),
                status: "loaded".into(),
                detail: "non-cooperative".into(),
            }],
        }
    }
}

impl DocsStore for FixtureStore {
    fn load_rows(&self, view: DocsView) -> Vec<DocsRow> {
        match view {
            DocsView::Documents => vec![DocsRow {
                id: "doc".into(),
                title: "Policy pack".into(),
                status: "indexed".into(),
                summary: "12 chunks".into(),
            }],
            DocsView::Evidence => vec![DocsRow {
                id: "claim".into(),
                title: "Evidence".into(),
                status: "verified".into(),
                summary: "citation chain".into(),
            }],
        }
    }
}

impl LearningStore for FixtureStore {
    fn load_rows(&self, view: LearningView) -> Vec<LearningRow> {
        match view {
            LearningView::Proposals => vec![LearningRow {
                id: "proposal".into(),
                kind: "prompt-rule".into(),
                state: "pending".into(),
                evidence: "3 events".into(),
            }],
            LearningView::Manifests => vec![LearningRow {
                id: "manifest".into(),
                kind: "agent-policy".into(),
                state: "active".into(),
                evidence: "v2".into(),
            }],
            LearningView::Incidents => vec![LearningRow {
                id: "incident".into(),
                kind: "false-completion".into(),
                state: "open".into(),
                evidence: "claim mismatch".into(),
            }],
        }
    }
}

impl WorkflowStore for FixtureStore {
    fn load_rows(&self, view: &WorkflowView) -> Vec<WorkflowRow> {
        match view {
            WorkflowView::Runs => vec![WorkflowRow {
                id: "wf-run".into(),
                label: "Dynamic audit".into(),
                status: "completed".into(),
                detail: "4/4 accepted".into(),
            }],
            WorkflowView::RunDetail { run_id } => vec![WorkflowRow {
                id: run_id.clone(),
                label: "quality-gate".into(),
                status: "failed".into(),
                detail: "failure remains inspectable".into(),
            }],
        }
    }
}

#[test]
fn gametheory_docs_and_learning_screens_load_source_rows() {
    let store = FixtureStore;
    let mut gt = GameTheoryScreen::run_detail("gt-source");
    let mut docs = DocsScreen::evidence();
    let mut learning = LearningScreen::incidents();
    let mut workflow = WorkflowScreen::run_detail("wf-source");

    gt.load_from(&store);
    docs.load_from(&store);
    learning.load_from(&store);
    workflow.load_from(&store);

    assert_eq!(gt.selected().unwrap().id, "gt-source");
    assert_eq!(docs.selected().unwrap().status, "verified");
    assert_eq!(learning.selected().unwrap().kind, "false-completion");
    assert_eq!(workflow.selected().unwrap().status, "failed");
}

#[test]
fn evidence_engine_screens_render_source_rows_to_buffer() {
    let store = FixtureStore;
    let theme = intj_theme();

    let mut gt = GameTheoryScreen::run_detail("gt-source");
    gt.load_from(&store);
    let gt_rendered = render_screen(|frame| gt.render(frame, frame.area(), &theme));
    assert!(gt_rendered.contains("Game-Theory Run gt-source"));
    assert!(gt_rendered.contains("nash-equilibrium-finder"));

    let mut docs = DocsScreen::evidence();
    docs.load_from(&store);
    let docs_rendered = render_screen(|frame| docs.render(frame, frame.area(), &theme));
    assert!(docs_rendered.contains("Evidence"));
    assert!(docs_rendered.contains("citation chain"));

    let mut learning = LearningScreen::incidents();
    learning.load_from(&store);
    let learning_rendered = render_screen(|frame| learning.render(frame, frame.area(), &theme));
    assert!(learning_rendered.contains("Completion Incidents"));
    assert!(learning_rendered.contains("claim mismatch"));

    let mut workflow = WorkflowScreen::run_detail("wf-source");
    workflow.load_from(&store);
    let workflow_rendered = render_screen(|frame| workflow.render(frame, frame.area(), &theme));
    assert!(workflow_rendered.contains("Workflow wf-source"));
    assert!(workflow_rendered.contains("failure remains inspectable"));
}

#[test]
fn video_screen_renders_sources_progress_frames_and_summary() {
    let theme = intj_theme();
    let mut video = VideoScreen::sources();
    video.set_source_rows(vec![VideoSourceItem {
        video_id: "video-1".into(),
        title: "Trading lecture".into(),
        status: "indexed".into(),
        detail: "transcript and frames".into(),
    }]);
    video.set_transcript_segments(vec![TranscriptSegmentItem {
        start_ms: 12_000,
        end_ms: 15_000,
        text: "Impulse wave starts here".into(),
        speaker: Some("analyst".into()),
    }]);
    video.set_frame_rows(vec![FrameGroupItem {
        start_ms: 18_000,
        end_ms: 21_000,
        image_path: "frames/chart-001.jpg".into(),
        detail: "chart annotation".into(),
    }]);
    video.on_progress(7, "OCR batch complete".into(), "processing".into());
    video.set_summary("Elliott wave setup with visual evidence");

    let rendered = render_screen(|frame| video.render(frame, frame.area(), &theme));

    assert!(rendered.contains("Trading lecture"));
    assert!(rendered.contains("processing"));
    assert!(rendered.contains("Impulse wave"));
    assert!(rendered.contains("frames/chart-001"));
    assert!(rendered.contains("Elliott wave setup"));
}

fn render_screen(render: impl FnOnce(&mut ratatui::Frame)) -> String {
    let backend = TestBackend::new(96, 20);
    let mut terminal = Terminal::new(backend).expect("build TestBackend terminal");
    terminal.draw(render).expect("draw evidence screen");
    buffer_to_string(&terminal)
}

fn buffer_to_string(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    let mut rendered = String::with_capacity((area.width as usize + 1) * area.height as usize);
    for y in 0..area.height {
        for x in 0..area.width {
            rendered.push_str(buffer[(x, y)].symbol());
        }
        rendered.push('\n');
    }
    rendered
}
