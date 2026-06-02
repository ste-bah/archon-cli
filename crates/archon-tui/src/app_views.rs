use crate::app::App;
use crate::events::{EvidenceRowPayload, VideoIngestProgressEvent, ViewId};
use crate::evidence_view_state::EvidenceViewState;

impl App {
    pub fn open_view(&mut self, view_id: ViewId) {
        self.evidence_view = match view_id {
            ViewId::Docs => Some(EvidenceViewState::Docs(
                crate::screens::docs::DocsScreen::documents(),
            )),
            ViewId::Cognitive => Some(EvidenceViewState::Cognitive(
                crate::screens::cognitive::CognitiveScreen::executive(),
            )),
            ViewId::GameTheory => Some(EvidenceViewState::GameTheory(
                crate::screens::gametheory::GameTheoryScreen::main(),
            )),
            ViewId::Learning => Some(EvidenceViewState::Learning(
                crate::screens::learning::LearningScreen::proposals(),
            )),
            ViewId::Video => Some(EvidenceViewState::Video(
                crate::screens::video::VideoScreen::sources(),
            )),
            ViewId::Workflow => Some(EvidenceViewState::Workflow(
                crate::screens::workflow::WorkflowScreen::runs(),
            )),
            _ => None,
        };
    }

    pub fn open_view_with_rows(&mut self, view_id: ViewId, rows: Vec<EvidenceRowPayload>) {
        self.open_view(view_id);
        match self.evidence_view.as_mut() {
            Some(EvidenceViewState::Docs(screen)) => {
                screen.set_rows(
                    rows.into_iter()
                        .map(|row| crate::screens::docs::DocsRow {
                            id: row.id,
                            title: row.title,
                            status: row.status,
                            summary: row.detail,
                        })
                        .collect(),
                );
            }
            Some(EvidenceViewState::Cognitive(screen)) => {
                screen.set_rows(
                    rows.into_iter()
                        .map(|row| crate::screens::cognitive::CognitiveRow {
                            id: row.id,
                            label: row.title,
                            status: row.status,
                            detail: row.detail,
                        })
                        .collect(),
                );
            }
            Some(EvidenceViewState::GameTheory(screen)) => {
                screen.set_rows(
                    rows.into_iter()
                        .map(|row| crate::screens::gametheory::GameTheoryRow {
                            id: row.id,
                            label: row.title,
                            status: row.status,
                            detail: row.detail,
                        })
                        .collect(),
                );
            }
            Some(EvidenceViewState::Learning(screen)) => {
                screen.set_rows(
                    rows.into_iter()
                        .map(|row| crate::screens::learning::LearningRow {
                            id: row.id,
                            kind: row.title,
                            state: row.status,
                            evidence: row.detail,
                        })
                        .collect(),
                );
            }
            Some(EvidenceViewState::Video(screen)) => {
                screen.set_source_rows(
                    rows.into_iter()
                        .map(|row| crate::screens::video::VideoSourceItem {
                            video_id: row.id,
                            title: row.title,
                            status: row.status,
                            detail: row.detail,
                        })
                        .collect(),
                );
            }
            Some(EvidenceViewState::Workflow(screen)) => {
                screen.set_rows(
                    rows.into_iter()
                        .map(|row| crate::screens::workflow::WorkflowRow {
                            id: row.id,
                            label: row.title,
                            status: row.status,
                            detail: row.detail,
                        })
                        .collect(),
                );
            }
            None => {}
        }
    }

    pub fn on_video_ingest_progress(&mut self, event: VideoIngestProgressEvent) {
        if !matches!(self.evidence_view, Some(EvidenceViewState::Video(_))) {
            self.open_view(ViewId::Video);
        }
        if let Some(EvidenceViewState::Video(screen)) = self.evidence_view.as_mut() {
            screen.on_progress(event.segment_count, event.latest_text, event.status);
        }
    }
}
