# TASK-TDL-090 — Backtest Data Gates

```yaml
task_id: TASK-TDL-090
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: Backtest Data Gates
workstream: W3 Backtest Gates
complexity: medium
status: blocked
depends_on: ['TASK-TDL-020', 'TASK-TDL-080']
blocks: ['TASK-TDL-100', 'TASK-TDL-130']
source_sections: ['10', '29']
implements: [REQ-BT-001, REQ-BT-002, REQ-BT-003, REQ-BT-004]
required_env_keys: []
required_tools: []
deliverable_contracts: []
```

## Purpose

Wire production backtest refusal gates for invalid, degraded, non-native, or incomplete datasets.

## Scope

### In

- Dataset id/version based lookup.
- Fail-closed checks for missing dataset, validation, metadata fields, checksums, normalized file, raw artifact.
- `--diagnostic-allow-degraded-data` exploratory override behavior.

### Out

- AHDM strategy algorithm implementation.
- Live trading enablement.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- AC-AHDM-003 foundation and AC-DL-005 satisfied for backtests.
- Diagnostic override never satisfies promotion gates.
- Reports list every overridden dataset issue.
- Loose file paths are refused for promotion backtests.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test -p archon-trading backtest_invalid_dataset_refusal` — backtest refuses invalid datasets.
- `cargo test -p archon-trading backtest_diagnostic_override_marking` — diagnostic override report marking.
- `cargo test -p archon-trading backtest_checksum_mismatch_refusal` — checksum mismatch refusal.
- `cargo test -p archon-trading backtest_loose_path_refusal` — loose path refusal.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `needs_review` (semantic)

> TASK-TDL-090 functional backtest data gates are present and focused tests passed with non-zero matches, but I cannot accept the task because the repository worktree contains uncommitted changes outside the task's declared/likely ownership, so ownership cleanliness is not verified.

**Findings:** none. The mandatory reviews inspect only ACCEPTED tasks, so a task that was
never accepted produced no findings. Absence of findings here is absence of review, not a
clean bill of health.

<!-- PRIOR-RUN-FINDINGS:END -->

## Required Task Checklist

- implements (normative requirement IDs)
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
