/// SONA-specific errors.
#[derive(Debug, Clone)]
pub enum SonaError {
    TrajectoryValidation(String),
    WeightUpdate(String),
    DriftExceeded(String),
    FeedbackValidation(String),
    WeightPersistence(String),
    Checkpoint(String),
    RollbackLoop(String),
}

impl std::fmt::Display for SonaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TrajectoryValidation(s) => write!(f, "trajectory validation: {}", s),
            Self::WeightUpdate(s) => write!(f, "weight update: {}", s),
            Self::DriftExceeded(s) => write!(f, "drift exceeded: {}", s),
            Self::FeedbackValidation(s) => write!(f, "feedback validation: {}", s),
            Self::WeightPersistence(s) => write!(f, "weight persistence: {}", s),
            Self::Checkpoint(s) => write!(f, "checkpoint: {}", s),
            Self::RollbackLoop(s) => write!(f, "rollback loop: {}", s),
        }
    }
}

impl std::error::Error for SonaError {}
