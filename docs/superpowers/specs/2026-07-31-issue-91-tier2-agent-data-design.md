# Issue #91 Tier-2 Agent Data Refactor Design

## Goal

Close core-audit finding 39 by splitting three oversized Rust production files without changing behavior, public APIs, initialization, ordering, serialization, lookup, or failure semantics:

- `crates/archon-pipeline/src/coding/agents.rs`
- `crates/archon-pipeline/src/research/agents.rs`
- `crates/archon-core/src/agents/catalog.rs`

This is a motion-only refactor. Existing catalog quirks are characterized and tracked separately, not corrected here.

## Constraints

- Preserve every current public item type after verifying it in source.
- Preserve exact agent definitions and source order.
- Preserve compile-time/static initialization; no `LazyLock`, `Vec`, runtime concatenation, parser, or data-file loader.
- Keep every changed/new Rust file below 500 lines.
- Keep every changed/new function strictly below 50 lines.
- Follow existing sibling test-file convention using `#[path = "*_tests.rs"]` where tests are moved.
- Do not mix unrelated cleanup or behavior fixes into the refactor.

## Architecture

### Coding definitions

`coding/agents.rs` remains the public façade containing the public data types, serialization support, public statics, and lookup helpers.

Private phase modules expose named `pub(super) const` agent values. The façade owns one explicit ordered backing array:

```rust
static AGENT_DEFINITIONS: [CodingAgent; 50] = [
    phase1::FIRST_AGENT,
    phase1::SECOND_AGENT,
    // Exact current order.
];

pub static AGENTS: &[CodingAgent] = &AGENT_DEFINITIONS;
```

The exact public declaration will be verified from current source before motion and retained byte-for-byte where practical. The fixed-size backing array makes omissions compile-time errors while preserving the public slice type.

### Research definitions

`research/agents.rs` remains the public façade containing public types, serialization support, public statics, and lookup/validation helpers.

Private phase modules expose named `pub(super) const` `ResearchAgent` values. A single explicit backing array preserves the exact current 47-agent execution sequence and existing public slice type.

Research phase metadata moves to a separate bounded module. `RESEARCH_PHASES`, phase membership, phase-list ordering, agent index semantics, and predecessor/successor relationships remain unchanged.

### Discovery catalog

`agents/catalog.rs` becomes a thin table of contents. Its explicitly ordered and responsibility-commented declarations/re-exports lead readers to:

- public catalog types and errors
- state, insertion, and index maintenance
- version and dependency resolution
- filtering, listing, and suggestions
- snapshots
- sibling catalog tests

Internal visibility changes only as required for modules to cooperate. Public paths, method signatures, concurrency primitives, ordering, error text, and current catalog quirks remain unchanged.

## Characterization Before Motion

Characterization tests land in their own commit and pass against the unsplit source. They remain read-only in all three motion commits. Any required characterization-test edit during motion is a stop signal indicating semantic drift.

### Coding coverage

- exact 50-key ordered sequence
- full serialized fingerprint of every definition
- lookup and phase-filter ordering
- dependency targets and order
- model, prompt path, tools, algorithms, memory keys, XP, access, parallel, critical, and description fields

### Research coverage

- exact 47-key ordered sequence
- full serialized fingerprint of every agent and phase definition
- positional lookup and representative/all neighbor relationships
- exact phase membership and phase-order subsequences
- tools, file/display metadata, memory keys, artifacts, and descriptions

### Catalog coverage

Characterize deterministic behavior only. Any map-derived result is sorted in the test before comparison, with a comment explaining that raw map iteration order is not contractual.

Tests cover current externally observable behavior for insertion/indexing, listing/filter ordering, version/dependency resolution, snapshots, errors, and source metadata. Existing unresolved-dependency and stale-index quirks receive dedicated GitHub issues. Characterization comments reference those issue numbers and explicitly say not to fix them in #91.

## Commit Sequence

1. **Characterization commit** — tests only, green against unsplit files.
2. **Coding motion commit** — phase modules plus explicit ordered backing array; characterization tests untouched.
3. **Research motion commit** — phase modules, metadata module, explicit ordered backing array; characterization tests untouched.
4. **Catalog motion commit** — responsibility modules and sibling test file; characterization tests untouched.

The issue may use additional correction commits only if review finds a refactor defect. No commit may combine a catalog quirk fix.

## Data Assets

Embedded validated assets are sequenced, not rejected. This motion-only split makes each typed phase module a natural future conversion unit. Asset migration remains separate because it changes parsing, validation, static-lifetime, initialization, and failure behavior.

## Verification

For each commit:

- targeted affected tests
- characterization tests unchanged and green
- relevant crate check and strict Clippy
- format check
- file/function size checks

Final verification:

- complete affected crate suites
- workspace check
- strict workspace Clippy with warnings denied
- workspace formatting
- source-of-truth definition diff/fingerprint comparison
- independent adversarial review
- executable smoke checks for public lookup, scheduling order, research adjacency, and catalog queries
- all six dev-flow gates

## Review Rule

Characterization tests are read-only in motion commits. Reviewers compare each explicit aggregation array against the pre-motion ordered-key baseline and reject any definition, ordering, public-surface, serialization, or failure-semantic drift.
