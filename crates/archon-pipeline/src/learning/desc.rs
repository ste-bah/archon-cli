//! DESC Episode Store - episode CRUD, injection filtering, quality monitoring.
//!
//! Implements REQ-LEARN-008. Native Rust - no UCM daemon dependency.
//! All operations use direct CozoDB calls against `desc_episodes` and
//! `desc_episode_metadata` relations initialized by `schema::initialize_learning_schemas`.

mod confidence;
mod helpers;
mod injection;
mod quality;
mod store;
mod trajectory;
mod types;

pub use confidence::ConfidenceCalculator;
pub use injection::InjectionFilter;
pub use quality::QualityMonitor;
pub use store::DescEpisodeStore;
pub use trajectory::TrajectoryLinker;
pub use types::{
    DEFAULT_MAX_INJECTION, DEFAULT_MIN_INJECTION_QUALITY, DEFAULT_QUALITY_THRESHOLD, DescEpisode,
    EpisodeQuery, InjectionResult, QualityReport, RankedEpisode,
};
