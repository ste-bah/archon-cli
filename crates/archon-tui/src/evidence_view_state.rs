use crate::events::ViewId;

/// Active Evidence Engine inspection overlay.
pub enum EvidenceViewState {
    Docs(crate::screens::docs::DocsScreen),
    Cognitive(crate::screens::cognitive::CognitiveScreen),
    GameTheory(crate::screens::gametheory::GameTheoryScreen),
    Learning(crate::screens::learning::LearningScreen),
    Video(crate::screens::video::VideoScreen),
    Workflow(crate::screens::workflow::WorkflowScreen),
}

impl EvidenceViewState {
    pub fn view_id(&self) -> ViewId {
        match self {
            Self::Docs(_) => ViewId::Docs,
            Self::Cognitive(_) => ViewId::Cognitive,
            Self::GameTheory(_) => ViewId::GameTheory,
            Self::Learning(_) => ViewId::Learning,
            Self::Video(_) => ViewId::Video,
            Self::Workflow(_) => ViewId::Workflow,
        }
    }
}
