# TASK-TDL-001 — Data Lake Gap Audit

```yaml
task_id: TASK-TDL-001
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: Data Lake Gap Audit
workstream: W0 Audit
complexity: medium
status: ready
depends_on: []
blocks: ['TASK-TDL-010']
source_sections: ['2', '5', '14', '17', '24']
required_env_keys: []
required_tools: []
deliverable_contracts:
  - kind: audit_report
    artifact_path: .archon/trading-lab/data/gap-audit-current.json
```

## Purpose

Inspect the existing Trading Lab store, registry, commands, providers, docs, and tests before implementation.

## Scope

### In

- Read current `.archon/trading-lab/data` layout.
- Inspect `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, and `src/command/trading_data.rs`.
- Identify existing tests and missing focused tests.
- Produce a gap report in the task notes or existing context file.

### Out

- No schema migration implementation.
- No provider implementation.


## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- Gap report maps current code and missing implementation to every normative PRD requirement: `REQ-DL-001..133`, `REQ-AHDM-001..033`, `REQ-BT-001..004`, acceptance criteria, command contract, storage contract, provider-specific contracts, migration plan, residual-gap policy, and done definition.
- Existing registry behavior is documented honestly, including zero-dataset state if still true.
- No storage-root change is proposed.

## Focused Tests

- Existing focused data-store tests, if present.
- `cargo check --workspace --tests -j1` when code is touched; otherwise no compile required.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `accepted`

> Verified TASK-TDL-001 as an audit-only artifact task: required gap audit artifact exists and parses, maps all required normative IDs, documents non-zero registry state honestly, preserves the existing storage root, and repository source tree has no uncommitted changes. One focused data-store test passed from the repository root.

**Unremediated findings against this task (5, 4 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `F1` · high · adversarial review**

- *claim:* Accepted verification treats 170 unique IDs as satisfying normative requirement mapping.
- *counter evidence:* Task requires mapping current code and missing implementation to every normative PRD requirement; artifact sample has repeated generic evidence for REQ-DL-001..004.
- *source:* task:51-56; gap-audit-current.json:366-390

**F2 · `F2` · medium · adversarial review**

- *claim:* Implementation wrote/generated the audit artifact.
- *counter evidence:* Recorded generation command contains only a comment, not the generator logic; patch file is zero-line/empty.
- *source:* implement result commands_run[5]; patch read error

**F3 · `F3` · medium · adversarial review**

- *claim:* Verification result has residual_gaps: [].
- *counter evidence:* Implementation result and artifact record four fail-closed residual gaps.
- *source:* verification-wave result; gap-audit-current.json:2468-2493

**F4 · `F4` · medium · adversarial review**

- *claim:* Storage contract paths are present.
- *counter evidence:* Artifact marks wildcard/template paths like datasets/<dataset-id>/<version>/metadata.json as present with 'Observed or contract-required', not direct physical evidence.
- *source:* gap-audit-current.json:2527-2567

**F5 · `F5` · low · adversarial review**

- *claim:* No files changed / source tree clean proves task safety.
- *counter evidence:* The deliverable is a project artifact outside repository source tracking, so git status does not prove artifact diff integrity.
- *source:* implementation and verification commands_run

<!-- PRIOR-RUN-FINDINGS:END -->

## Required Task Checklist

- scope
- files expected to change
- files forbidden to change
- acceptance criteria
- focused tests
- line-count check
- complexity check where applicable
- adversarial review notes
- explicit residual gaps with fail-closed behavior

## Global Constraints

- Keep changed/new files under 500 lines where possible.
- No hardcoded secrets or provider credentials.
- No production candle resampling.
- No vague "later", "TBD", "probably", or "best effort" without a residual gap record.
