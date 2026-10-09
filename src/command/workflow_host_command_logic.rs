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
//! - Every outcome records the version that judged it ([`LOGIC_VERSION_STAMP`]),
//!   the pinned digest of its source ([`LOGIC_DIGEST_STAMP`]) and the build
//!   that judged it ([`LOGIC_BUILD_STAMP`]).
//!   An outcome without a version stamp was written before logic versions,
//!   by some earlier binary (any one, from before b22 too), so a runtime
//!   transition lies between it and this read; what it was judged by is
//!   unknown. [`outcome_logic_holds`] says when such a record may still
//!   answer.
//! - The checks that are cheap to run again (verify-frozen-* and
//!   requirements-trace) also need the same build: a record replays only
//!   when it was stamped by this very binary ([`THIS_BUILD`], the build
//!   fingerprint of every source file, as the critic findings store uses).
//!   Any binary change re-runs them, whatever the version and the pinned
//!   digest say, and a record without a build stamp never replays one. This
//!   is the backstop for a logic change the version guard missed, and for a
//!   build that shipped a stale pin. task-set-lint (its critic run is
//!   costly) and the landings keep version-only reuse; whether they also
//!   bind to the build is Steven's decision.
//!
//! A developer cannot change a capability's logic and forget the version:
//! `sources_digest` pins every source file its verdict is built from (the
//! dependency closure of its roots, `workflow_host_command_logic_closure`,
//! minus [`LOGIC_DENYLIST`]), and the test
//! `logic_361_sources_match_their_pinned_digest` fails when one changes.
//! Comment lines and in-file test blocks are not hashed. The fix is to bump
//! `version` when the change can alter any verdict, or to re-pin the digest
//! at the same version when it cannot; either way it is one reviewed line
//! here.
//!
//! A version bump of freeze-acceptance or freeze-skeleton never re-judges an
//! artifact already frozen on disk: the script calls only
//! verify-frozen-* for a stage the launcher finds frozen. It re-keys the
//! submissions still in flight; only the set gates re-judge what is frozen.
use archon_workflow::{CommandCapability, WorkflowError};

/// The version every capability has until its logic first changes after
/// logic versions were introduced. Its key is the key the capability had
/// before them. A record without a version stamp was not necessarily judged
/// by version-1 logic: it can come from any earlier binary. Version 1 stays
/// the freeze baseline anyway: a bump would not re-judge a frozen artifact
/// (see the module doc) and would re-run an in-flight freeze for nothing.
pub(crate) const BASELINE_LOGIC_VERSION: u32 = 1;

/// Where a host command's outcome records the logic version that judged it.
pub(crate) const LOGIC_VERSION_STAMP: &str = "logicVersion";

/// Where a host command's outcome records the digest of the logic that
/// judged it.
pub(crate) const LOGIC_DIGEST_STAMP: &str = "logicDigest";

/// Where a host command's outcome records the build that judged it.
pub(crate) const LOGIC_BUILD_STAMP: &str = "logicBuild";

/// This binary's build fingerprint (`build.rs`): it changes with any
/// source, manifest, toolchain or profile change.
pub(crate) const THIS_BUILD: &str = env!("ARCHON_BUILD_FINGERPRINT");

/// The reuse-key token that carries a version above the baseline.
pub(crate) const LOGIC_VERSION_TOKEN: &str = "CAPABILITY_LOGIC_VERSION";

/// One capability's logic: its version, the root modules of its verdict
/// (the guard hashes their whole dependency closure, tests and denied files
/// aside), and the digest of that closure.
#[derive(Debug)]
pub(crate) struct CapabilityLogic {
    pub(crate) id: &'static str,
    pub(crate) version: u32,
    /// Read only by the source-digest guard test.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by the guard test"))]
    pub(crate) sources: &'static [&'static [&'static str]],
    /// Stamped into every outcome ([`LOGIC_DIGEST_STAMP`]).
    pub(crate) sources_digest: &'static str,
    /// A record replays only when this very build ([`THIS_BUILD`]) judged
    /// it as well.
    pub(crate) build_bound: bool,
}

/// Files the closure of a capability reaches that no verdict depends on,
/// each with the reason. A file here can change without a logic-version
/// decision, so an entry is a reviewed line; the guard test fails on an
/// entry no capability reaches.
#[cfg(test)]
pub(crate) const LOGIC_DENYLIST: &[(&str, &str)] = &[
    (
        "crates/archon-llm/",
        "LLM provider transport and model catalogue. The catalogue resolves the critic's \
         model alias, so a remap changes which model judges task-set-lint: that is a \
         model choice, not logic. It re-runs no recorded verdict; a lint that runs anyway \
         asks the critic again, as its findings store keys on the resolved model. The \
         request and its reply reader are hashed",
    ),
    (
        "src/command/pipeline_workflow_llm.rs",
        "adapter from the workflow LLM port to the pipeline runner: transport only",
    ),
    (
        "crates/archon-tools/",
        "the tools agents call at run time; the lint reads tool names and permissions, \
         never a tool's behaviour",
    ),
    (
        "crates/archon-mcp/",
        "MCP server transport and lifecycle; the permitted tool list is read by hashed code",
    ),
    (
        "crates/archon-memory/",
        "memory store client, reached only through agent configuration",
    ),
    (
        "crates/archon-core/src/agent/",
        "interactive agent loop, reached through configuration types",
    ),
    (
        "crates/archon-core/src/agents/",
        "agent definition registry, reached through configuration types",
    ),
    (
        "crates/archon-core/src/subagent/",
        "subagent runner and retention, reached through configuration types",
    ),
    (
        "crates/archon-core/src/sandbox/",
        "container and SSH sandbox backends the configuration names",
    ),
    (
        "crates/archon-core/src/orchestrator/",
        "agent pool, reached through configuration types",
    ),
    (
        "src/command/requirement_trace/slash.rs",
        "the interactive /requirements front end; the host command runs the CLI path",
    ),
];

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
    "src/command/topology_lint/repository_claims.rs",
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
        // 3: check verdict reuse now binds check text, bounded repository
        // tree, environment, this binary, and the logic digest.
        version: 3,
        sources: &[GATE, CONTRACT, FREEZE],
        sources_digest: "41efa6d4e621bf54284bc887d67c80898157ef7ce5a8b496fca7400a794b12cb",
        build_bound: false,
    },
    CapabilityLogic {
        id: "freeze-skeleton",
        version: 1,
        sources: &[GATE, CONTRACT, FREEZE],
        sources_digest: "41efa6d4e621bf54284bc887d67c80898157ef7ce5a8b496fca7400a794b12cb",
        build_bound: false,
    },
    CapabilityLogic {
        id: "land-task-body",
        version: 2,
        sources: &[GATE, CONTRACT, LINT],
        sources_digest: "28956fd725b5ae41e215342221d1dee03ea1e42fa3316401af2a5e46250e5ebc",
        build_bound: false,
    },
    CapabilityLogic {
        id: "task-set-lint",
        version: 2,
        sources: &[GATE, CONTRACT, LINT],
        sources_digest: "28956fd725b5ae41e215342221d1dee03ea1e42fa3316401af2a5e46250e5ebc",
        build_bound: false,
    },
    CapabilityLogic {
        id: "requirements-trace",
        version: 1,
        sources: &[GATE, CONTRACT, TRACE],
        sources_digest: "ad204011e0a687e2c4ae8231c08fa8efcc364e7386ca9a69a828732c7c28f7f2",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-acceptance",
        version: 1,
        sources: &[GATE, CONTRACT, VERIFY],
        sources_digest: "b87c122bfeea232b328ea2f7b6a71a890fb656eed2d08b3d146fecc8f4865928",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-skeleton",
        version: 1,
        sources: &[GATE, CONTRACT, VERIFY],
        sources_digest: "b87c122bfeea232b328ea2f7b6a71a890fb656eed2d08b3d146fecc8f4865928",
        build_bound: true,
    },
];

/// Every capability's logic version, by id.
pub(crate) fn versions() -> std::collections::BTreeMap<String, u32> {
    CAPABILITY_LOGIC
        .iter()
        .map(|logic| (logic.id.to_string(), logic.version))
        .collect()
}

/// Every capability's pinned logic digest, by id, and whether its reuse is
/// bound to the build.
pub(crate) fn digests() -> std::collections::BTreeMap<String, (String, bool)> {
    CAPABILITY_LOGIC
        .iter()
        .map(|logic| {
            let digest = (logic.sources_digest.to_string(), logic.build_bound);
            (logic.id.to_string(), digest)
        })
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

/// Whether a recorded outcome may answer again under logic `current`, run
/// by build `bound` when the capability's reuse is bound to the build.
///
/// A stamped outcome answers only under the version it records. An
/// unstamped one predates logic versions and was written by another binary:
/// - a check (`judges_only`) never answers; it runs again and is stamped.
///   Its verdict is all it produced, and the binary that judged it is gone;
/// - a capability that published an artifact answers while its logic is
///   still the baseline: its artifact is on disk, the checks after it run
///   again on this build, and its key changes with its first version bump.
///
/// A build-bound outcome also needs the build `bound` stamped: one judged
/// by another binary, or by one that stamped no build, runs again. The
/// pinned digest is not compared: it moves only when a developer re-pins.
pub(crate) fn outcome_logic_holds(
    data: &serde_json::Value,
    current: u32,
    checks: bool,
    bound: Option<&str>,
) -> bool {
    let version = match data.get(LOGIC_VERSION_STAMP) {
        Some(stamp) => stamp.as_u64() == Some(u64::from(current)),
        None => !checks && current == BASELINE_LOGIC_VERSION,
    };
    let build = bound.is_none_or(|build| {
        data.get(LOGIC_BUILD_STAMP)
            .and_then(serde_json::Value::as_str)
            == Some(build)
    });
    version && build
}
