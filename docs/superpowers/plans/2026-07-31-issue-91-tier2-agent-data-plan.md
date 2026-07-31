# Issue #91 Tier-2 Agent Data Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split three oversized Rust production files into bounded, responsibility-focused modules while preserving every observable definition, order, API, initialization, serialization, lookup, and catalog behavior.

**Architecture:** Commit immutable characterization baselines against the unsplit source first. Then perform three independent motion commits: coding phase definitions, research phase definitions/metadata, and catalog responsibilities. Typed compile-time constants and explicit fixed-size backing arrays preserve static semantics and reviewable order; characterization tests remain byte-unchanged through all motion commits.

**Tech Stack:** Rust 2024, Serde/serde_json, SHA-256 or stable checked-in serialized fingerprints, Tokio/DashMap/ArcSwap, Cargo test/check/Clippy/rustfmt, GitHub issues, Archon six-gate dev flow.

## Global Constraints

- Work only in `/tmp/archon-issue91-tier2-agent-data` on `audit/issue91-tier2-agent-data`.
- Preserve current public item types after verifying them directly in source; sketches yield to source.
- Characterization expectations are hardcoded from the unsplit source and never derived from actual runtime values in the assertion path.
- Characterization tests land in their own commit before motion and remain read-only in all three motion commits.
- Preserve compile-time/static initialization; no `LazyLock`, `Vec` aggregation, runtime concatenation, parser, or data-file loader.
- Preserve exact coding and research definition order.
- Preserve current catalog quirks in #91; file separate issues and reference them in characterization comments.
- Sort map-derived sequences in tests before comparison; raw map iteration order is not contractual.
- Every changed/new Rust file must be below 500 lines.
- Every changed/new function must be strictly below 50 lines.
- Catalog tests use the sibling `#[path = "catalog_tests.rs"]` convention.
- Do not modify protected local-main worktree `/home/unixdude/Archon-projects/archon/project-work/archon-cli`.
- Use `CARGO_BUILD_JOBS=1` and `--test-threads=1` for bounded verification.
- Use no AI/Claude/Codex authorship in commits.

---

## File Structure

### Characterization

- Modify `crates/archon-pipeline/tests/coding_agents.rs`: hardcoded ordered keys, full serialized fingerprints, phase order, dependencies.
- Modify `crates/archon-pipeline/tests/research_agents.rs`: hardcoded ordered keys/fingerprints, phase metadata, adjacency/order.
- Create `crates/archon-core/tests/catalog_characterization.rs`: public-API catalog quirk and deterministic-behavior characterizations, keeping the unsplit 909-line production file untouched in the tests-only commit.

### Coding motion

- Modify `crates/archon-pipeline/src/coding/agents.rs`: retain types, serde, explicit 50-item aggregation, helpers.
- Create `crates/archon-pipeline/src/coding/agent_definitions/mod.rs`: phase module table of contents.
- Create `crates/archon-pipeline/src/coding/agent_definitions/understanding.rs`.
- Create `crates/archon-pipeline/src/coding/agent_definitions/design.rs`.
- Create `crates/archon-pipeline/src/coding/agent_definitions/wiring_plan.rs`.
- Create `crates/archon-pipeline/src/coding/agent_definitions/implementation.rs`.
- Create `crates/archon-pipeline/src/coding/agent_definitions/testing.rs`.
- Create `crates/archon-pipeline/src/coding/agent_definitions/refinement.rs`.

### Research motion

- Modify `crates/archon-pipeline/src/research/agents.rs`: retain types, serde, explicit 47-item aggregation, public statics/helpers.
- Create `crates/archon-pipeline/src/research/agent_definitions/mod.rs`.
- Create `crates/archon-pipeline/src/research/agent_definitions/foundation.rs`.
- Create `crates/archon-pipeline/src/research/agent_definitions/discovery.rs`.
- Create `crates/archon-pipeline/src/research/agent_definitions/architecture.rs`.
- Create `crates/archon-pipeline/src/research/agent_definitions/synthesis.rs`.
- Create `crates/archon-pipeline/src/research/agent_definitions/design.rs`.
- Create `crates/archon-pipeline/src/research/agent_definitions/writing.rs`.
- Create `crates/archon-pipeline/src/research/agent_definitions/validation.rs`.
- Create `crates/archon-pipeline/src/research/agent_definitions/final_assembly.rs`.
- Create `crates/archon-pipeline/src/research/phase_definitions.rs`.

### Catalog motion

- Modify `crates/archon-core/src/agents/catalog.rs`: ordered responsibility-commented façade and public re-exports.
- Create `crates/archon-core/src/agents/catalog_types.rs`: filters, views, keys, snapshots, source config/kind, errors/conversions.
- Create `crates/archon-core/src/agents/catalog_state.rs`: `DiscoveryCatalog`, constructor/default, insertion, indexes, snapshots.
- Create `crates/archon-core/src/agents/catalog_resolution.rs`: version and dependency resolution.
- Create `crates/archon-core/src/agents/catalog_query.rs`: names, listing, filtering, search, suggestions.
- Create `crates/archon-core/src/agents/catalog_tests.rs`: existing and characterization tests moved unchanged.

---

### Task 1: Commit Immutable Pre-Motion Characterization

**Files:**
- Modify: `crates/archon-pipeline/tests/coding_agents.rs`
- Modify: `crates/archon-pipeline/tests/research_agents.rs`
- Create: `crates/archon-core/tests/catalog_characterization.rs`

**Interfaces:**
- Consumes: current unsplit `AGENTS`, `RESEARCH_AGENTS`, `RESEARCH_PHASES`, `DiscoveryCatalog` APIs.
- Produces: checked-in hardcoded baselines that motion commits must satisfy without test edits.

- [ ] **Step 1: Verify public static types and capture unsplit baselines**

Read exact declarations and use a one-off script or temporary ignored test to print canonical JSON plus ordered keys. Record, but do not commit, the generation script output. Confirm source declarations currently are:

```rust
pub static AGENTS: &[CodingAgent]
pub static RESEARCH_AGENTS: &[ResearchAgent]
pub static RESEARCH_PHASES: &[ResearchPhase]
```

Expected counts: coding 50, research 47.

- [ ] **Step 2: Add hardcoded coding baseline tests**

Add constants containing the complete ordered 50-key list and the complete canonical `serde_json::to_string` output for every definition. The committed constants contain literal strings copied from one unsplit-source generation run. Assertion code compares runtime serialization to those literals; it must not generate expected values from `AGENTS`.

The test shape is:

```rust
#[test]
fn coding_definitions_match_pre_split_baseline() {
    let keys = AGENTS.iter().map(|agent| agent.key).collect::<Vec<_>>();
    assert_eq!(keys.as_slice(), EXPECTED_CODING_KEYS);

    let serialized = AGENTS
        .iter()
        .map(|agent| serde_json::to_string(agent).expect("serialize coding agent"))
        .collect::<Vec<_>>();
    assert_eq!(serialized.as_slice(), EXPECTED_CODING_JSON);
}
```

`EXPECTED_CODING_KEYS` and `EXPECTED_CODING_JSON` must be full checked-in literal arrays with lengths 50. Generate them once from the unsplit source, copy the output into the test, delete the generator, then run the committed assertion. This uses existing `serde_json` support and adds no dependency.

- [ ] **Step 3: Add hardcoded research baseline tests**

Add complete literal arrays for all 47 ordered keys, all 47 canonical serialized agent JSON strings, every canonical serialized phase JSON string, and expected phase key subsequences. Assert every key appears exactly once across `RESEARCH_PHASES`, phase subsequences preserve execution order, `get_agent_index` equals array position, and predecessor/successor pairs match adjacent entries.

```rust
#[test]
fn research_definitions_match_pre_split_baseline() {
    assert_eq!(
        RESEARCH_AGENTS.iter().map(|agent| agent.key).collect::<Vec<_>>().as_slice(),
        EXPECTED_RESEARCH_KEYS,
    );
    assert_eq!(
        RESEARCH_AGENTS
            .iter()
            .map(|agent| serde_json::to_string(agent).expect("serialize research agent"))
            .collect::<Vec<_>>()
            .as_slice(),
        EXPECTED_RESEARCH_JSON,
    );
}
```

The expected arrays are generated once from unsplit source, pasted as literals, and never regenerated in the assertion path.

- [ ] **Step 4: File catalog quirk issues before characterizing them**

The approved follow-up issues are:

1. unresolved catalog dependencies are silently omitted during dependency resolution: #107;
2. same-path catalog replacement can leave stale tag/capability index memberships: #108.

Do not fix either behavior in #91.

- [ ] **Step 5: Add deterministic catalog characterization**

Add tests to new integration test `crates/archon-core/tests/catalog_characterization.rs`, using only current public catalog APIs. This keeps the oversized unsplit production file untouched in the tests-only commit. Reference the approved issue numbers in comments:

```rust
// Characterizes quirk tracked in #107; do not fix in #91.
#[tokio::test]
async fn unresolved_dependencies_are_currently_omitted() {
    let resolved = catalog.resolve_with_dependencies("root", None).await.unwrap();
    assert_eq!(resolved.iter().map(|agent| agent.name.as_str()).collect::<Vec<_>>(), ["root"]);
}

// Characterizes quirk tracked in #108; do not fix in #91.
#[tokio::test]
async fn replacement_preserves_current_stale_index_behavior() {
    let mut names = catalog.list(&filter_for_old_tag()).await
        .into_iter().map(|agent| agent.name).collect::<Vec<_>>();
    // DashMap iteration order is not contractual; sort before comparing.
    names.sort();
    assert_eq!(names, ["replacement-agent"]);
}
```

Use the actual existing catalog fixture constructors and public method names read from `catalog.rs`; assertions above pin the required literal outcomes. Do not introduce production helpers solely for these tests.

Also characterize deterministic listing/version/source/error behavior required by the spec.

- [ ] **Step 6: Run characterization tests against unsplit files**

Run:

```bash
CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline --test coding_agents -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline --test research_agents -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test -p archon-core --test catalog_characterization -- --test-threads=1
```

Expected: all new and existing tests pass while all three production files remain unsplit.

- [ ] **Step 7: Verify tests-only scope and commit**

Run `git diff --name-only`; only the two existing pipeline test files and new core integration test may appear. Commit:

```bash
git add crates/archon-pipeline/tests/coding_agents.rs \
  crates/archon-pipeline/tests/research_agents.rs \
  crates/archon-core/tests/catalog_characterization.rs
git commit -m "test(audit): freeze Tier-2 agent definitions"
```

Commit description must say fingerprints and ordered lists were generated once from unsplit source and hardcoded, and are read-only in the next three commits.

---

### Task 2: Split Coding Definitions by Phase

**Files:**
- Modify: `crates/archon-pipeline/src/coding/agents.rs`
- Create: `crates/archon-pipeline/src/coding/agent_definitions/mod.rs`
- Create: six phase files listed in File Structure
- Test unchanged: `crates/archon-pipeline/tests/coding_agents.rs`

**Interfaces:**
- Consumes: `CodingAgent`, `Phase`, `ToolAccess`, `Algorithm` from façade.
- Produces: named `pub(super) const` definitions and private `[CodingAgent; 50]` backing array exposed through unchanged `pub static AGENTS: &[CodingAgent]`.

- [ ] **Step 1: Confirm characterization tests are clean and unchanged**

Record `git diff HEAD~0 -- crates/archon-pipeline/tests/coding_agents.rs` as empty before motion. Copy its blob SHA for later comparison:

```bash
git hash-object crates/archon-pipeline/tests/coding_agents.rs
```

- [ ] **Step 2: Create phase modules with named constants**

Move each complete `CodingAgent` literal from `agents.rs:166-1339`, unchanged, into the phase module matching its `Phase`. Use explicit names derived from keys. For example, `contract-agent` becomes `CONTRACT_AGENT`; its initializer is the exact source block currently at `agents.rs:167-185`:

```rust
use super::super::{Algorithm, CodingAgent, Phase, ToolAccess};

pub(super) const CONTRACT_AGENT: CodingAgent = CodingAgent {
    key: "contract-agent",
    phase: Phase::Understanding,
    model: "sonnet",
    prompt_source_path: ".archon/agents/coding-pipeline/contract-agent.md",
    tool_access: ToolAccess::ReadOnly,
    algorithm: Algorithm::ToT,
    fallback_algorithm: Some(Algorithm::Reflexion),
    depends_on: &[],
    memory_reads: &["coding/input/task", "coding/context/project"],
    memory_writes: &[
        "coding/understanding/task-analysis",
        "coding/understanding/parsed-intent",
    ],
    xp_reward: 50,
    parallelizable: false,
    critical: true,
    description: "Parses and structures coding requests into actionable components. CRITICAL agent - pipeline entry point.",
};
```

Keep each file below 500 lines. If one semantic phase exceeds budget, split it into responsibility-named sibling modules while retaining one phase-level table of contents; never create arbitrary line-range chunks.

- [ ] **Step 3: Add explicit aggregation table**

In `agents.rs`, declare private modules and construct an explicit 50-item array. Its entries, in order, are the uppercase constant names generated from these literal keys:

```text
contract-agent, requirement-extractor, requirement-prioritizer, scope-definer,
context-gatherer, feasibility-analyzer, pattern-explorer, technology-scout,
research-planner, codebase-analyzer, phase-1-reviewer, phase-2-reviewer,
system-designer, component-designer, interface-designer, data-architect,
integration-architect, wiring-obligation-agent, phase-3-reviewer, code-generator,
type-implementer, unit-implementer, service-implementer, data-layer-implementer,
api-implementer, frontend-implementer, error-handler-implementer, config-implementer,
logger-implementer, integration-verification-agent, dependency-manager,
implementation-coordinator, phase-4-reviewer, test-generator, test-runner,
integration-tester, regression-tester, security-tester, coverage-analyzer, quality-gate,
test-fixer, phase-5-reviewer, performance-optimizer, performance-architect,
code-quality-improver, security-architect, final-refactorer, sign-off-approver,
phase-6-reviewer, recovery-agent
```

Declare `mod agent_definitions;`, then write `static AGENT_DEFINITIONS: [CodingAgent; 50]` by mapping every hyphenated key in the complete sequence above to its uppercase underscore constant in the module corresponding to the agent's current `Phase`. Expose it with the current public declaration, unchanged:

```rust
pub static AGENTS: &[CodingAgent] = &AGENT_DEFINITIONS;
```

The fixed array length and checked-in ordered-key test jointly reject omission, duplication, or reordering.

Retain public types, serde implementation, helper signatures, docs, and public static declaration type.

- [ ] **Step 4: Run coding characterization and affected suite**

```bash
CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline --test coding_agents -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline coding:: -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo check -p archon-pipeline
CARGO_BUILD_JOBS=1 cargo clippy -p archon-pipeline --all-targets -- -D warnings
cargo fmt --all --check
```

Expected: pass with no characterization changes.

- [ ] **Step 5: Verify preservation and size**

- Compare characterization test blob SHA to Step 1; must match.
- Verify every new/changed Rust file below 500 lines.
- Verify every changed function below 50 lines.
- Inspect explicit 50-entry array against `EXPECTED_CODING_KEYS`.

- [ ] **Step 6: Commit coding motion**

```bash
git add crates/archon-pipeline/src/coding/agents.rs \
  crates/archon-pipeline/src/coding/agent_definitions
git commit -m "refactor(pipeline): split coding agent definitions"
```

Commit body: typed phase modules are the safe motion-only step and natural units for a future separately governed data-asset migration.

---

### Task 3: Split Research Definitions and Phase Metadata

**Files:**
- Modify: `crates/archon-pipeline/src/research/agents.rs`
- Create: `crates/archon-pipeline/src/research/agent_definitions/mod.rs`
- Create: eight phase files listed in File Structure
- Create: `crates/archon-pipeline/src/research/phase_definitions.rs`
- Test unchanged: `crates/archon-pipeline/tests/research_agents.rs`

**Interfaces:**
- Consumes: `ResearchAgent`, `ResearchPhase`, `ResearchToolAccess`, base/writer tool constants.
- Produces: named `pub(super) const` agent values, explicit `[ResearchAgent; 47]` aggregation, bounded phase metadata, unchanged public slices/helpers.

- [ ] **Step 1: Capture characterization test blob SHA**

```bash
git hash-object crates/archon-pipeline/tests/research_agents.rs
```

- [ ] **Step 2: Move exact agent literals to semantic phase modules**

Each module imports types/tool constants and exposes named constants. Move initializer blocks from `research/agents.rs:183-736` without editing their tokens except the new `pub(super) const NAME: ResearchAgent =` binding. For example, `step-back-analyzer` becomes `STEP_BACK_ANALYZER` in `foundation.rs`; copy its complete current initializer from `research/agents.rs:185-197`.

Preserve every string/slice and exact execution order.

- [ ] **Step 3: Move phase metadata unchanged**

`phase_definitions.rs` contains named `pub(super) const ResearchPhase` values or one fixed-size backing array. Preserve exact phase IDs, names, descriptions, agents, and ordering. Do not deduplicate or normalize current data.

- [ ] **Step 4: Add explicit façade aggregations**

```rust
static RESEARCH_AGENT_DEFINITIONS: [ResearchAgent; 47] = [
    agent_definitions::foundation::FIRST_AGENT,
    // Exact baseline order.
];

pub static RESEARCH_AGENTS: &[ResearchAgent] = &RESEARCH_AGENT_DEFINITIONS;
```

Adapt `RESEARCH_PHASES` around its verified current public type without changing it.

- [ ] **Step 5: Run research characterization and affected suite**

```bash
CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline --test research_agents -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline research:: -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo check -p archon-pipeline
CARGO_BUILD_JOBS=1 cargo clippy -p archon-pipeline --all-targets -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 6: Verify preservation and size**

- Characterization test blob SHA unchanged.
- All new/changed Rust files below 500 lines.
- All changed functions below 50 lines.
- Explicit array exactly matches `EXPECTED_RESEARCH_KEYS`.
- Research phase subsequences and adjacency tests pass.

- [ ] **Step 7: Commit research motion**

```bash
git add crates/archon-pipeline/src/research/agents.rs \
  crates/archon-pipeline/src/research/agent_definitions \
  crates/archon-pipeline/src/research/phase_definitions.rs
git commit -m "refactor(pipeline): split research agent definitions"
```

Commit body repeats that data assets are sequenced after behavior-preserving typed modules, not rejected.

---

### Task 4: Split Discovery Catalog by Responsibility

**Files:**
- Modify: `crates/archon-core/src/agents/catalog.rs`
- Create: `crates/archon-core/src/agents/catalog_types.rs`
- Create: `crates/archon-core/src/agents/catalog_state.rs`
- Create: `crates/archon-core/src/agents/catalog_resolution.rs`
- Create: `crates/archon-core/src/agents/catalog_query.rs`
- Create: `crates/archon-core/src/agents/catalog_tests.rs`

**Interfaces:**
- Consumes: current catalog public types/methods, DashMap/ArcSwap state, validation conversion.
- Produces: unchanged `crate::agents::catalog::*` public surface via explicitly ordered façade re-exports/delegating impls.

- [ ] **Step 1: Capture catalog characterization blob**

```bash
git hash-object crates/archon-core/src/agents/catalog.rs
```

Also record exact public declarations and method signatures from `catalog.rs` before motion.

- [ ] **Step 2: Extract public types and errors**

Move `FilterLogic`, `AgentFilter`, `AgentInfoView`, `AgentKey`, `CatalogSnapshot`, `DiscoveryError`, `DiscoverySourceConfig`, `DiscoverySourceKind`, clone/default/conversion implementations into `catalog_types.rs` unchanged. Use `pub` only where current items are public; otherwise `pub(super)`.

- [ ] **Step 3: Extract state, insertion, and snapshots**

Move `DiscoveryCatalog` storage, `new`, `insert`, snapshot logic, and `Default` to `catalog_state.rs`. Preserve DashMap/ArcSwap initialization and all index update behavior exactly, including stale-index quirk tracked by the new issue.

- [ ] **Step 4: Extract resolution**

Move exact-version/range resolution and dependency DFS to `catalog_resolution.rs`. Preserve highest-version selection, DFS output, cycle handling, and silent unresolved-dependency omission tracked by its issue.

- [ ] **Step 5: Extract query/filter behavior**

Move names, listing, filtering, searching, suggestions, and semver sorting to `catalog_query.rs`. Preserve deterministic explicit sorts and leave nondeterministic map iteration non-contractual.

- [ ] **Step 6: Build ordered façade table of contents**

`catalog.rs` explicitly declares/re-exports by responsibility. Preserve the currently public catalog symbols exactly: `AgentFilter`, `AgentInfoView`, `AgentKey`, `CatalogSnapshot`, `DiscoveryCatalog`, `DiscoveryError`, `DiscoverySourceConfig`, `DiscoverySourceKind`, and `FilterLogic`. Keep the façade ordered and commented:

```rust
// Public catalog data contracts and errors.
mod catalog_types;
pub use catalog_types::{
    AgentFilter, AgentInfoView, AgentKey, CatalogSnapshot, DiscoveryError,
    DiscoverySourceConfig, DiscoverySourceKind, FilterLogic,
};

// Concurrent state and secondary-index maintenance.
mod catalog_state;
pub use catalog_state::DiscoveryCatalog;

// Version/dependency resolution and read-side queries.
mod catalog_resolution;
mod catalog_query;

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
```

Because these sibling files live under `agents`, actual declarations may need `#[path = "catalog_types.rs"]` attributes or a `catalog/` directory. Choose the form that preserves `crate::agents::catalog::*` paths and follows current module resolution; do not change public paths.

- [ ] **Step 7: Move tests without semantic edits**

Move the complete existing/characterization test module to `catalog_tests.rs`. Only imports/module qualification needed for compilation may change. Quirk comments and assertions remain semantically unchanged.

- [ ] **Step 8: Run catalog and core verification**

```bash
CARGO_BUILD_JOBS=1 cargo test -p archon-core agents::catalog -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test -p archon-core -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo check -p archon-core
CARGO_BUILD_JOBS=1 cargo clippy -p archon-core --all-targets -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 9: Verify public surface, behavior, and size**

- Compare pre/post public declarations and method signatures.
- Confirm characterization semantics unchanged.
- Confirm every file below 500 lines and function below 50 lines.
- Confirm façade comments/re-exports are explicitly responsibility ordered.

- [ ] **Step 10: Commit catalog motion**

```bash
git add crates/archon-core/src/agents/catalog.rs \
  crates/archon-core/src/agents/catalog_types.rs \
  crates/archon-core/src/agents/catalog_state.rs \
  crates/archon-core/src/agents/catalog_resolution.rs \
  crates/archon-core/src/agents/catalog_query.rs \
  crates/archon-core/src/agents/catalog_tests.rs
git commit -m "refactor(core): split discovery catalog responsibilities"
```

---

### Task 5: Final Verification, Review, Gates, and Shipment

**Files:**
- Verify all files changed by Tasks 1–4.
- Gate records under `.gates/ISSUE-91` are evidence only and remain uncommitted unless repository policy explicitly tracks them.

**Interfaces:**
- Consumes: four commits and separate quirk issue references.
- Produces: verified/pushed issue branch, cumulative integration, closed GitHub #91.

- [ ] **Step 1: Prove characterization tests were read-only during motion**

Use `git diff` between the characterization commit and final head for the two pipeline characterization files and semantic test content moved from catalog. Any baseline expectation change blocks shipment.

- [ ] **Step 2: Run complete affected suites**

```bash
CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline -- --test-threads=1
CARGO_BUILD_JOBS=1 cargo test -p archon-core -- --test-threads=1
```

Record exact test-case counts, failures, and ignored tests by test binary; do not conflate summaries/assertions/files.

- [ ] **Step 3: Run workspace mechanical checks**

```bash
CARGO_BUILD_JOBS=1 cargo check --workspace --all-targets
CARGO_BUILD_JOBS=1 cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

- [ ] **Step 4: Run size and definition-preservation checks**

Run repository file-size guard and a function-size scan over changed Rust files. Independently compare ordered arrays/fingerprints against checked-in pre-motion baselines.

- [ ] **Step 5: Run bounded adversarial review**

Reviewer must independently read the final source and all four commits. Required checks:

- public API/type preservation
- exact definition/order/fingerprint preservation
- static initialization
- catalog behavior/quirk preservation
- characterization tests untouched
- file/function budgets
- no unrelated changes

Verdict must be `APPROVED` before Gate 3/6 evidence.

- [ ] **Step 6: Execute smoke checks**

Use exact qualified tests or a small existing CLI invocation proving:

- coding key lookup and phase scheduling order
- research positional lookup and predecessor/successor order
- catalog insertion, resolution, sorted listing, and snapshot

Each smoke command must execute nonzero test counts and exit 0.

- [ ] **Step 7: Pass all six gates in order**

Use:

```bash
/home/unixdude/Archon-projects/archon/scripts/dev-flow-pass-gate.sh ISSUE-91 <gate> <evidence> /tmp/archon-issue91-tier2-agent-data
/home/unixdude/Archon-projects/archon/scripts/dev-flow-gate.sh ISSUE-91 /tmp/archon-issue91-tier2-agent-data
```

Gate 1 cites the pre-motion characterization commit. Gate 5 uses `--exec`. Gate 3 and Gate 6 cite explicit `APPROVED` verdicts.

- [ ] **Step 8: Push, integrate, close, clean**

1. Push `audit/issue91-tier2-agent-data`.
2. Synchronize cumulative worktree with latest remote cumulative branch without touching protected local-main.
3. Cherry-pick the four #91 commits into `audit/core-remediation-2026-07`.
4. Push cumulative branch and verify remote SHA.
5. Comment on and close GitHub #91 with exact commits, test counts, gate evidence, quirk issue links, and preservation statement.
6. Remove #91 worktree and prune.
7. Store completion in MemoryGraph.
8. Mark task complete only after all six gates and remote verification.
