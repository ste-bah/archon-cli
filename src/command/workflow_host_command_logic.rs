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
    "src/command/topology_lint/fidelity_critic.rs",
    "src/command/topology_lint/fidelity_store.rs",
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
        // 2: #366 live-root rule (sibling paths pass; a bad repository lock
        // is an error) changes acceptance freeze verdicts.
        // 3: #288 acceptance checks rerun only when their input closure
        // changed; a reused check verdict binds normalized literal paths,
        // their filesystem state, the shell, environment and this logic.
        // 4: #380 complete wrapped obligation text changes freeze verdicts.
        // #385's re-ask improves recovery from an unusable provider reply;
        // it does not change the verdict for a usable reply. #392 changes
        // which authored reply shapes can supply an acceptance entry.
        // #392 accepts a sole unnamed check block when command_block is omitted.
        // #399 fixes resume seed reconstruction; identical gate input keeps its verdict.
        // 9: #400 validates incremental subsets independently, allowing supplementary-only updates to pass.
        // 7: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 10: #374 resolves filesystem aliases without refusing unrelated paths on the same volume.
        // 11: #417 #418 #419 freeze-owned outputs leave check inputs; passability and judge verdicts reuse by scoped input keys.
        // #421 changes when a recorded freeze is reused (a later publication extends its pin), not how it is judged.
        // 12: #423 check blocks normalize CR/CRLF to LF; a shell setup error is refused as a pre-implementation proof.
        // #434 re-pin only: the fidelity critic's store and reply schema are in this closure; freeze never asks the critic.
        // 14: #417 task-root exclusions and copies use the same project-relative input set as its digest.
        // #417 re-pin only: direct copy calls now normalize roots like the production caller.
        // 15: #417 Windows task-root paths normalize before containment checks, so probes can run and produce a different verdict.
        // #439 re-pin only: resume phase selection shares this closure; freeze verdicts are unchanged.
        // #417 re-pin only: the landing destination allowlist names the topology stores; freeze verdicts are unchanged.
        // #425 re-pin only: the repository claim parser changed; freeze verdicts do not read claims.
        // #434c re-pin only: validated fidelity-cache reuse does not alter frozen verdicts.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        // 16: #440 the scratch copy skips a repository nested inside a project input, as the probe digest does, so a check can no longer read that live tree and its verdict can change.
        // 17: #420 a pre-implementation failure at a setup path outside the probe's tree that the output reports absent (`cd X`, `--manifest-path X`) gives no verdict: unproven, then its author's, never a passability verdict.
        // #420 review, same unreleased version 17: the rule applies only in a host-made copy (scratch, hermetic), never on the live tree, and a top-level name the copy holds (`../repo`) stays inside it.
        // 18: #446 direct-site baseline runs use their hermetic copy's layout when applying that rule.
        // #446 review 2, same unreleased version 18
        version: 18,
        sources: &[GATE, CONTRACT, FREEZE],
        // #434d re-pin only: the fidelity critic keeps each passing verdict of a reply and its store keys on a critic logic version; freeze never asks the critic, so its verdicts are unchanged.
        // #446 follow-up re-pin only: cache compatibility now keeps old evidence;
        // Scratch schema-3 verdicts stay reusable, while Hermetic layoutless
        // evidence reruns under the unchanged version-18 acceptance pin.
        sources_digest: "712d76bf89e1af57927f275b45014d70343156b095aabc543115aba99f2ae091",
        build_bound: false,
    },
    CapabilityLogic {
        id: "freeze-skeleton",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // #434 and #417 re-pin only: harness input paths, pipeline outputs, and copy API normalization do not alter this verdict.
        // #439 re-pin only, as freeze-acceptance.
        // #425 re-pin only: the repository claim parser changed; skeleton freeze verdicts do not read claims.
        // #434c re-pin only: validated fidelity-cache reuse does not alter frozen verdicts.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        version: 3,
        sources: &[GATE, CONTRACT, FREEZE],
        // #434d re-pin only: the fidelity critic keeps each passing verdict of a reply and its store keys on a critic logic version; freeze never asks the critic, so its verdicts are unchanged.
        // #440 re-pin only: the skeleton freeze runs no check and copies no input.
        // #420 re-pin only: the skeleton freeze judges no check run, so the new no-verdict rule never applies.
        // #440 #420 re-pin only: line-ceiling layout of the scratch and hermetic modules (imports, a re-export); no logic change.
        // #446 re-pin only: the skeleton freeze judges no baseline run.
        // #446 follow-up re-pin only: the shared probe cache source changed;
        // freeze-skeleton judges no check runs.
        sources_digest: "712d76bf89e1af57927f275b45014d70343156b095aabc543115aba99f2ae091",
        build_bound: false,
    },
    CapabilityLogic {
        id: "land-task-body",
        // 3: R8 body packaging strips chat around the one frozen subject's task
        // file and binds by its task_id; the fidelity critic re-asks on an
        // unparseable answer while it makes progress.
        // 4: #380's obligation model changes the frozen contract verdict.
        // 5: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 6: #425 repository claims recognize leading negation and explicit directory scopes.
        // 7: #428 the host stamps frozen skeleton frontmatter fields at landing.
        // 8: #434 a fidelity gap blocks only its weakest task's body; true verdicts quote a checked anchor and are reused per obligation.
        // 9: #425 dotted identifiers followed by a file citation are not repository-path claims; shared-scope negation and tree-aware dotted or single-word scope resolution now preserve claim polarity and identify the actual scoped path.
        // #434c re-pin only: identical critic logic and request identities reuse validated verdicts across binary rebuilds.
        version: 9,
        sources: &[GATE, CONTRACT, LINT],
        // #425 re-pin only: unscoped negated bare names, filler before a scoped predicate, base-name, slashed and multi-directory scopes (each directory one reading); same version bump as above.
        // #434d re-pin only: a fidelity reply is read verdict by verdict and each verdict kept passes every check a whole reply passes; the store keys on CRITIC_LOGIC_VERSION 1 and migrates schema-4/5 records, so no verdict changes meaning.
        // #434d review re-pin only: a build whose critic sources are not the pinned ones keeps no fidelity record and asks afresh; an opposite re-answer of a kept verdict is only logged. Verdict meaning is unchanged.
        sources_digest: "771d5f0e009ff4d4fffb9cc6421a1fac4c2cf6434b541af698e9f22cd367fe39",
        build_bound: false,
    },
    CapabilityLogic {
        id: "task-set-lint",
        // 3: the fidelity critic re-asks on an unparseable answer while it
        // makes progress (R8).
        // 4: #380's obligation model changes the frozen contract verdict.
        // 5: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 6: #425 repository claims recognize leading negation and explicit directory scopes.
        // 7: #434 true fidelity verdicts quote a checked anchor and are reused per obligation over byte-identical cluster texts.
        // 8: #425 dotted identifiers followed by a file citation are not repository-path claims; shared-scope negation and tree-aware dotted or single-word scope resolution now preserve claim polarity and identify the actual scoped path.
        // #434c re-pin only: identical critic logic and request identities reuse validated verdicts across binary rebuilds.
        version: 8,
        sources: &[GATE, CONTRACT, LINT],
        // #425 re-pin only: unscoped negated bare names, filler before a scoped predicate, base-name, slashed and multi-directory scopes (each directory one reading); same version bump as above.
        // #434d re-pin only: a fidelity reply is read verdict by verdict and each verdict kept passes every check a whole reply passes; the store keys on CRITIC_LOGIC_VERSION 1 and migrates schema-4/5 records, so no verdict changes meaning.
        // #434d review re-pin only: a build whose critic sources are not the pinned ones keeps no fidelity record and asks afresh; an opposite re-answer of a kept verdict is only logged. Verdict meaning is unchanged.
        sources_digest: "771d5f0e009ff4d4fffb9cc6421a1fac4c2cf6434b541af698e9f22cd367fe39",
        build_bound: false,
    },
    CapabilityLogic {
        id: "requirements-trace",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        version: 3,
        sources: &[GATE, CONTRACT, TRACE],
        // #434 and #417 re-pin only: trace never reads the newly excluded run state.
        // #434d re-pin only: trace never asks the fidelity critic or reads its store.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        // #440 re-pin only: trace calls none of the scratch copy, audit or probe digest walkers that changed.
        // #440 #420 re-pin only: line-ceiling layout of the scratch and hermetic modules (imports, a re-export); no logic change.
        sources_digest: "48ddc3dedc35ac89d1b741fd15c47a2da799e24626624ca675dc375dfadd2d6e",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-acceptance",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        // 4: #417 copied check inputs omit freeze-owned files, which can change a check verdict.
        // 5: #417 harness-written project run state leaves hermetic check inputs, which can change a check verdict.
        // 6: #417 copied task-root inputs now match the digest exclusions, which can change a check verdict.
        // #417 re-pin only: the direct copy API now shares the production caller's root normalization.
        // #434c re-pin only: validated fidelity-cache reuse does not alter frozen-chain verification verdicts.
        version: 6,
        sources: &[GATE, CONTRACT, VERIFY],
        // #434 and #417 re-pin only: skeleton verification never reads excluded run state or pipeline outputs.
        // #439 re-pin only: resume phase selection does not change verification verdicts.
        // #425 re-pin only: the repository claim parser changed; acceptance verification verdicts do not read claims.
        // #434d re-pin only: verification never asks the fidelity critic or reads its store.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        // #440 re-pin only: frozen-chain verification runs no check and calls none of the changed copy, audit or digest walkers.
        // #420 re-pin only: frozen-chain verification judges no check run, so the new no-verdict rule never applies.
        // #420 review re-pin only: rustfmt layout of the executability modules; no logic change.
        // #440 #420 re-pin only: line-ceiling layout of the scratch and hermetic modules (imports, a re-export); no logic change.
        // #446 re-pin only: baseline judgment changes; frozen-artifact verification does not.
        // #446 follow-up re-pin only: verification shares cache source files
        // but reads no probe result layout or evidence.
        sources_digest: "d9eee0149bfb6521548db9dbcaa8b3e9c53bd97b0e587c38f2c37dd921e21312",
        build_bound: true,
    },
    CapabilityLogic {
        id: "verify-frozen-skeleton",
        // 3: #386 an output overrun no longer fails the command; the verdict uses the full output.
        version: 3,
        sources: &[GATE, CONTRACT, VERIFY],
        // #434 and #417 re-pin only: verify never asks the fidelity critic or reads pipeline outputs.
        // #439 re-pin only: resume phase selection does not change verification verdicts.
        // #425 re-pin only: the repository claim parser changed; skeleton verification verdicts do not read claims.
        // #434c re-pin only: validated fidelity-cache reuse does not alter frozen-chain verification verdicts.
        // #434d re-pin only: verification never asks the fidelity critic or reads its store.
        // #438 re-pin only: relative path keys use '/' on every OS; macOS/Linux keys and verdicts are unchanged.
        // #440 re-pin only: frozen-chain verification runs no check and calls none of the changed copy, audit or digest walkers.
        // #420 re-pin only: frozen-chain verification judges no check run, so the new no-verdict rule never applies.
        // #420 review re-pin only: rustfmt layout of the executability modules; no logic change.
        // #440 #420 re-pin only: line-ceiling layout of the scratch and hermetic modules (imports, a re-export); no logic change.
        // #446 re-pin only: baseline judgment changes; frozen-artifact verification does not.
        // #446 follow-up re-pin only: verification shares cache source files
        // but reads no probe result layout or evidence.
        sources_digest: "d9eee0149bfb6521548db9dbcaa8b3e9c53bd97b0e587c38f2c37dd921e21312",
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
