//! The logic version of the archon subcommand each host command runs (Issue 361).
//!
//! A host command's reuse key names what the call does (argv, stdin,
//! environment, writes, policy) and what it was given. It did not name how
//! the subcommand judges it, so a binary that made a freeze, lint, trace or
//! verify stricter replayed the verdict of the binary before it. Each
//! capability therefore carries a logic version here:
//!
//! - The version is in the key ([`key_token`]), so a bump re-keys that
//!   capability alone and its next call runs. A binary upgrade that changes
//!   no capability's logic keeps every key and reuses every result.
//! - [`BASELINE_LOGIC_VERSION`] adds nothing to the key, so a capability
//!   whose logic never changed keeps the keys the records written before
//!   logic versions already hold.
//! - Every outcome records the version that judged it ([`LOGIC_VERSION_STAMP`]).
//!   An outcome without one was written before logic versions, by a binary
//!   that is not this one, so a runtime transition lies between it and this
//!   read; what it was judged by is unknown. [`outcome_logic_holds`] says
//!   when such a record may still answer.
//!
//! A developer cannot change a capability's logic and forget the version:
//! `sources_digest` pins the source files its verdict is built from, and the
//! test `logic_361_sources_match_their_pinned_digest` fails when they change.
//! The fix is to bump `version` when the change can alter any verdict, or to
//! re-pin the digest at the same version when it cannot; either way it is
//! one reviewed line here.
use archon_workflow::{CommandCapability, WorkflowError};

/// The version every capability had when logic versions were introduced.
/// Its key is the key the capability had before them.
pub(crate) const BASELINE_LOGIC_VERSION: u32 = 1;

/// Where a host command's outcome records the logic version that judged it.
pub(crate) const LOGIC_VERSION_STAMP: &str = "logicVersion";

/// The reuse-key token that carries a version above the baseline.
pub(crate) const LOGIC_VERSION_TOKEN: &str = "CAPABILITY_LOGIC_VERSION";

/// One capability's logic: its version, and the source files (module roots,
/// each with its whole module subtree except tests) that version stands for.
#[derive(Debug)]
pub(crate) struct CapabilityLogic {
    pub(crate) id: &'static str,
    pub(crate) version: u32,
    /// Read only by the source-digest guard test.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the guard test"))]
    pub(crate) sources: &'static [&'static [&'static str]],
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the guard test"))]
    pub(crate) sources_digest: &'static str,
}

/// How every gate builds and reads its envelope.
const GATE: &[&str] = &[
    "src/command/workflow_gate.rs",
    "src/command/workflow_gate_envelope.rs",
    "crates/archon-workflow/src/v2/gate_envelope.rs",
];

/// The frozen contract, skeleton and obligation model every gate checks.
const CONTRACT: &[&str] = &[
    "crates/archon-workflow/src/task_set_contract.rs",
    "crates/archon-workflow/src/task_skeleton.rs",
    "crates/archon-workflow/src/task_set_edges.rs",
    "crates/archon-workflow/src/obligation_ids.rs",
];

/// `workflow freeze-acceptance` and `workflow freeze-skeleton`.
const FREEZE: &[&str] = &[
    "src/command/workflow_freeze_cli.rs",
    "src/command/workflow_freeze_acceptance_cli.rs",
    "src/command/workflow_task_set.rs",
    "src/command/workflow_freeze_candidate.rs",
    "src/command/workflow_freeze_shape.rs",
    "src/command/workflow_freeze_schema.rs",
    "src/command/workflow_freeze_defects.rs",
    "src/command/workflow_freeze_entry_validator.rs",
    "src/command/workflow_freeze_marker_defects.rs",
    "src/command/workflow_freeze_staged_output.rs",
    "src/command/workflow_task_set_candidate.rs",
    "src/command/acceptance_chain.rs",
];

/// `workflow lint`, for one task file and for the whole set.
const LINT: &[&str] = &[
    "src/command/workflow_staged_cli.rs",
    "src/command/topology_lint.rs",
    "src/command/topology_task_graph.rs",
    "crates/archon-workflow/src/fidelity_audit.rs",
    "crates/archon-workflow/src/defect.rs",
];

/// `requirements trace`.
const TRACE: &[&str] = &[
    "src/command/requirement_trace.rs",
    "src/command/topology_task_graph.rs",
    "crates/archon-workflow/src/task_universe.rs",
    "crates/archon-workflow/src/repository_record.rs",
    "crates/archon-workflow/src/defect.rs",
];

/// `workflow verify-frozen-chain`.
const VERIFY: &[&str] = &["src/command/workflow_decompose_frozen_chain.rs"];

/// Every capability of the fixed catalog, and nothing else
/// (`logic_361_every_catalog_capability_has_a_logic_version`).
pub(crate) const CAPABILITY_LOGIC: &[CapabilityLogic] = &[
    CapabilityLogic {
        id: "freeze-acceptance",
        version: 1,
        sources: &[GATE, CONTRACT, FREEZE],
        sources_digest: "c394439dfde5304f4998cb71f016ea92fb5ab011b3b9df034a0c3f0907554a52",
    },
    CapabilityLogic {
        id: "freeze-skeleton",
        version: 1,
        sources: &[GATE, CONTRACT, FREEZE],
        sources_digest: "c394439dfde5304f4998cb71f016ea92fb5ab011b3b9df034a0c3f0907554a52",
    },
    CapabilityLogic {
        id: "land-task-body",
        version: 1,
        sources: &[GATE, CONTRACT, LINT],
        sources_digest: "140b9229c8622e55923aa818a3ec7b788c1e29ff13458222172d8c631798bef9",
    },
    CapabilityLogic {
        id: "task-set-lint",
        version: 1,
        sources: &[GATE, CONTRACT, LINT],
        sources_digest: "140b9229c8622e55923aa818a3ec7b788c1e29ff13458222172d8c631798bef9",
    },
    CapabilityLogic {
        id: "requirements-trace",
        version: 1,
        sources: &[GATE, CONTRACT, TRACE],
        sources_digest: "e28ced197e20b19bf92fbe31f9a9879aa629af986d9d79e5a47855aa88140eab",
    },
    CapabilityLogic {
        id: "verify-frozen-acceptance",
        version: 1,
        sources: &[GATE, CONTRACT, VERIFY],
        sources_digest: "55734b039dfd1897c2a127d3b3e08b1a1bf1f5ff94816e58586ca92ab5391871",
    },
    CapabilityLogic {
        id: "verify-frozen-skeleton",
        version: 1,
        sources: &[GATE, CONTRACT, VERIFY],
        sources_digest: "55734b039dfd1897c2a127d3b3e08b1a1bf1f5ff94816e58586ca92ab5391871",
    },
];

/// Every capability's logic version, by id.
pub(crate) fn versions() -> std::collections::BTreeMap<String, u32> {
    CAPABILITY_LOGIC
        .iter()
        .map(|logic| (logic.id.to_string(), logic.version))
        .collect()
}

/// A capability without a logic version names no key: the build is wrong,
/// and guessing a version could replay a verdict.
pub(crate) fn undeclared(id: &str) -> WorkflowError {
    WorkflowError::SpecInvalid(format!(
        "host command capability '{id}' has no logic version; declare it in CAPABILITY_LOGIC"
    ))
}

/// What the version adds to the reuse key: nothing at the baseline, so the
/// keys of records written before logic versions still match.
pub(crate) fn key_token(version: u32) -> Option<String> {
    (version != BASELINE_LOGIC_VERSION).then(|| version.to_string())
}

/// A capability that publishes nothing but its gate envelope: a check whose
/// whole product is its verdict (verify, lint and trace of the set).
pub(crate) fn judges_only(capability: &CommandCapability) -> bool {
    capability
        .declared_write_set
        .iter()
        .all(|write| write == "{GATE_ENVELOPE}")
}

/// Whether a recorded outcome may answer again under logic `current`.
///
/// A stamped outcome answers only under the version it records. An
/// unstamped one predates logic versions and was written by another binary:
/// - a check (`judges_only`) never answers; it runs again and is stamped.
///   Its verdict is all it produced, and the binary that judged it is gone;
/// - a capability that published an artifact answers while its logic is
///   still the baseline: its artifact is on disk, the checks after it run
///   again on this build, and its key changes with its first version bump.
pub(crate) fn outcome_logic_holds(data: &serde_json::Value, current: u32, checks: bool) -> bool {
    match data.get(LOGIC_VERSION_STAMP) {
        Some(stamp) => stamp.as_u64() == Some(u64::from(current)),
        None => !checks && current == BASELINE_LOGIC_VERSION,
    }
}
