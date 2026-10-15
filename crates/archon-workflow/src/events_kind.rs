use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEventKind {
    Started,
    StageStarted,
    ScriptPreflightRejected,
    StageCompleted,
    StageFailed,
    StageStalled,
    StageSkipped,
    ForcedAccepted,
    Resumed,
    Paused,
    Cancelled,
    Completed,
    LearningRecorded,
    DecompositionPhaseStarted,
    AuthorAttemptStarted,
    /// The provider request for an author attempt is in flight. Distinct from
    /// `AuthorAttemptStarted`, which marks the logical attempt beginning.
    ModelCallInFlight,
    AuthorAttemptCompleted,
    AuthorAttemptInterrupted,
    AuthorAttemptRejected,
    HostCommandStarted,
    HostCommandCompleted,
    ShadowFindingsObserved,
    SubjectAccepted,
    SubjectAcceptedWithShadowFindings,
    DecompositionPhaseCompleted,
    DecompositionCompleted,
    /// A fixed decomposition's current task bodies and frozen gates were
    /// observed again without publishing task files.
    DecompositionReverified,
    /// A fixed decomposition re-verification reserved this event sequence
    /// before writing any call records, so an interrupted retry gets new IDs.
    DecompositionReverifyStarted,
    /// A fixed decomposition resumed on a different build than the one that
    /// launched it (Issue-59). The persisted identity keeps the launch
    /// revision; this event, carrying `persisted` and `current`, is the record
    /// of the drift.
    BinaryRevisionDrift,
    /// A resume found the run `Running` while no live process held its
    /// executor lease: the previous owner died without a pause (Issue 251).
    /// Carries the lease evidence; the run moved to `Paused` with it.
    StaleOwnerRecovered,
    /// A damaged acceptance round record was moved aside with its evidence
    /// and the progress ledger rebuilt without it (Issue 262). Carries the
    /// record, where its bytes went, why, and whether its state was known.
    AcceptanceRecordQuarantined,
    /// The acceptance record the final gate is bound to was damaged or
    /// gone, and the gate was rebuilt from the acceptance call's own result
    /// (Issue 262, round 9). Carries the record, why, and the call.
    AcceptanceGateRebuilt,
    /// A live task body differed from its current landing receipt and was
    /// preserved in the run directory before reconciliation repaired it.
    LandingReceiptDisplaced,
    RunEndAcceptanceObserverStarted,
    RunEndAcceptanceShadowObserved,
    RunEndAcceptanceObserverFailed,
    /// One residual gap with `severity: "blocking"` was recorded against a
    /// call. Emitted once per distinct blocking gap, carrying the gap's id and
    /// description, so `events.jsonl` names the same blockers `v2/results/`
    /// does. See [`blocking_gap_events`].
    BlockingGapDetected,
    WriteCoordinationItemWritePlanCreated,
    WriteCoordinationWaveScheduled,
    WriteCoordinationItemWorkspaceCreated,
    WriteCoordinationUndeclaredWriteDetected,
    WriteCoordinationPatchCaptured,
    WriteCoordinationPatchApplied,
    WriteCoordinationPatchConflict,
    WriteCoordinationWaveVerificationResult,
    WriteCoordinationDirectCanonicalMutationDetected,
    WriteCoordinationSerialFallback,
    /// A kind this build does not know, written by a newer one. Reading
    /// it as `Unknown` keeps an older build able to read the log at all:
    /// every reader of `events.jsonl` parses whole lines, so one unknown
    /// kind used to fail a resume, a finalizer or a status read (Issue 251).
    #[serde(other)]
    Unknown,
}
