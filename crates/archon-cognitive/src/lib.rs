pub mod candidate_planner;
mod candidate_store;
pub mod cognitive_tick;
mod cognitive_tick_store;
pub mod config;
mod cozo_guard;
pub mod daemon;
pub mod dead_letters;
mod decision_codec;
pub mod decision_store;
pub mod executive_loop;
mod executive_support;
mod governed_apply_store;
pub mod governed_autonomous_apply;
pub mod inspection;
pub mod metrics;
pub mod policy_gate;
mod reflection_store;
pub mod reflection_trigger;
pub mod reflection_writer;
pub mod schema;
pub mod self_model;
pub mod shadow;
pub mod situation_classifier;
pub mod store;
pub mod tool_use_gate;
pub mod types;
pub mod verification_contracts;
pub mod world_model_scoring;

pub use archon_policy::CognitivePolicy;
pub use candidate_planner::{CandidatePlanner, HeuristicWeights};
pub use cognitive_tick::{CognitiveTick, TickReport};
pub use config::{CognitiveConfig, CognitiveDaemonConfig};
pub use daemon::{
    CognitiveDaemon, CognitiveTickJob, DaemonJob, DaemonJobReport, DaemonPaths, DaemonState,
    DaemonStatus,
};
pub use dead_letters::DeadLetterReplay;
pub use decision_store::DecisionStore;
pub use executive_loop::{
    ActionExecution, ActionExecutor, ActionOutcome, ExecutiveAdvisoryInput, ExecutiveLoop,
    ExecutiveRunOutcome, ExecutiveTurnInput, NoopActionExecutor, PlannedActionInput,
};
pub use governed_autonomous_apply::{
    ApplyResult, BehaviourManifestKind, CanaryOutcome, GovernedAutonomousApply, Proposal,
};
pub use inspection::{
    CognitiveInspection, CognitiveInspectionStatus, DecisionSummary, ProposalSummary,
    ReflectionSummary, ShadowSummary, TickSummary,
};
pub use metrics::{
    CognitiveMetricEvent, CognitiveMetricSnapshot, CohortRole, DerivedMetric, EvaluationWindow,
    METRIC_DEFINITION_VERSION, MetricCohort, MetricEmitter, MetricEventKind, MetricEventStore,
    MetricWriteOutcome, UNWINDOWED_EVALUATION_WINDOW, WindowDeclaration,
};
pub use policy_gate::{DenyReason, PolicyGate, PolicyVerdict, ProposalCheck, ProposalDenyReason};
pub use reflection_trigger::{
    HIGH_CONFIDENCE_CORRECTION_MIN, HIGH_SURPRISE_THRESHOLD, REPEATED_TOOL_FAILURE_THRESHOLD,
    ReflectionTrigger, TriggeredReflection, TurnSignals,
};
pub use reflection_writer::{
    LessonSink, NoopLessonSink, OutcomeSummary, ReflectInput, ReflectionRecord,
    ReflectionWriteOutcome, ReflectionWriter, TriggeredReflectInput,
};
pub use schema::{CURRENT_SCHEMA_VERSION, cognitive_schema_version, ensure_cognitive_schema};
pub use shadow::{
    LiveTurnOutcome, SHADOW_DEGRADED_MARKER, ShadowComparison, ShadowObservation, ShadowTurnInput,
    ShadowTurnObserver, observed_action_from_tools, surprise_of,
};
pub use situation_classifier::{ClassifyInput, SituationClassifier};
pub use store::{CognitiveStore, PersistentCognitiveStore};
pub use tool_use_gate::{ToolGateInput, ToolUseGate};
pub use types::{
    Candidate, CandidateActionKind, CandidateScore, ClassifierConfidence, CognitiveDecision,
    CognitiveError, CognitiveSurface, DecisionRecord, ExecutiveStateSnapshot, RejectedCandidate,
    RiskLevel, ScoreSource, Situation, SituationKind, ToolVerdict, direct_response_for,
};
pub use verification_contracts::{
    ContractInput, VerificationContract, VerificationEngine, VerificationEvidence,
    VerificationKind, VerificationRequirement, VerificationVerdict,
};
pub use world_model_scoring::{
    ModelKind, ModelPrediction, PredictionBackend, PredictionDimensions, ScoredCandidates,
    SharedPredictionBackend, WorldModelScorer, WorldModelState,
};
