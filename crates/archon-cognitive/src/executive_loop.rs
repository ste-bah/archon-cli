use std::path::{Path, PathBuf};

use archon_policy::CognitivePolicy;
use cozo::DbInstance;

use crate::executive_support::*;
use crate::self_model::{MemoryContext, SelfModelProfile, SelfModelStore};
use crate::world_model_scoring::NoopPredictionBackend;
use crate::{
    Candidate, CandidatePlanner, ClassifyInput, CognitiveConfig, CognitiveError, DecisionRecord,
    ExecutiveStateSnapshot, LessonSink, NoopLessonSink, OutcomeSummary, PolicyGate, ReflectInput,
    ReflectionWriter, Situation, SituationClassifier, SituationKind, VerificationContract,
    VerificationEngine, VerificationEvidence, VerificationVerdict, WorldModelScorer,
    WorldModelState,
};

include!("executive_loop_a.rs");
include!("executive_loop_b.rs");
