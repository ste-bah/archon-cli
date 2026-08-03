# TASK-TDL-140 — Adversarial Review + Paper Readiness

```yaml
task_id: TASK-TDL-140
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: Adversarial Review + Paper Readiness
workstream: W5 Backtest + Readiness
complexity: medium
status: blocked
depends_on: ['TASK-TDL-120', 'TASK-TDL-130']
blocks: []
source_sections: ['12', '27', '31', '32']
implements: [REQ-AHDM-009, REQ-AHDM-010, REQ-AHDM-030, REQ-AHDM-031, REQ-AHDM-032, REQ-AHDM-033]
required_env_keys: []
required_tools: [data_get_ohlcv, pine_analyze, pine_check, pine_compile, pine_smart_compile, pine_get_errors, pine_get_console]
deliverable_contracts:
  - kind: paper_trading_readiness
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/readiness/paper-trading-readiness.md
  - kind: adversarial_review
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/adversarial-review.md
```

## Purpose

Produce adversarial review and paper-trading readiness report with explicit pass/fail and residual gaps.

## Scope

### In

- KB evidence risk review.
- Data/provider/coverage risk review.
- Overfitting, slippage, execution, paper-readiness review.
- Residual gaps with fail-closed behavior.
- Paper-trading readiness artifact at `.archon/trading-lab/strategies/AHDM-v1/readiness/paper-trading-readiness.md`.
- Adversarial review artifact for each run at `.archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/adversarial-review.md`.

### Out

- Live trading approval.
- High-probability marketing claims.
- Vague residual gaps.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- AC-AHDM-004 satisfied.
- Done definition items are checked.
- Residual gaps use PRD §32 schema.
- Any failed gate blocks paper-readiness promotion.
- Writes `.archon/trading-lab/strategies/AHDM-v1/readiness/paper-trading-readiness.md`.
- Verifies each backtest run has `.archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/adversarial-review.md`.
- Failed gates are recorded in the readiness artifact and block readiness promotion.

## Focused Tests

Each bullet is a command a run can execute and a trace can match. A bullet
that only describes a test cannot promote a requirement past `Unproven`.

- `cargo test -p archon-trading readiness_report_structure` — readiness report structure.
- `cargo test -p archon-trading readiness_artifact_paths` — exact readiness and adversarial-review artifact paths.
- `cargo test -p archon-trading residual_gap_schema` — residual gap schema validation.
- `archon workflow lint --tasks tasks/PRD-TRADING-DATA-LAKE-AHDM-001` — the graph lints clean: diamond conformance, edge classification, stop-rule fusion and requirement coverage.
- `archon requirements trace --prd prds/PRD-TRADING-DATA-LAKE-AHDM-001/PRD-TRADING-DATA-LAKE-AHDM-001.md --tasks tasks/PRD-TRADING-DATA-LAKE-AHDM-001` — every `error`-severity requirement reaches at least `Exercised`. This is the gate the whole PRD is answerable to, so it is the last focused test.
- `cargo test -p archon-trading data_store` — the focused tests of every implementation task, re-run together.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `failed` — but for an INFRASTRUCTURE reason, not a judgement on the work:
> fanout branch 'verification-wave-verify-task-tdl-140-91-0' produced invalid structured output after repair

The verifier never rendered an opinion on this task. Treat the prior run as giving you no
information about correctness here — neither confidence nor suspicion.

**Unremediated findings against this task (5, 1 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `F1` · medium · adversarial review**

- *claim:* Declared TradingView/Pine tools were exercised and acceptance criteria satisfied.
- *evidence:* Required tools list has 7 entries, but remediation/verification commands mark pine_compile, pine_smart_compile, pine_get_errors, and pine_get_console failed exit_code=1 due Pine Editor unavailable; completion_evidence refs include only data_get_ohlcv, pine_analyze, and pine_check.
- *source:* TASK file lines 15; remediation JSON lines 107-132; verification JSON lines 117-142 and 240-242
- *verdict:* falsifies_overstatement

**F2 · `F2` · low · adversarial review**

- *claim:* Fresh remediation evidence artifact was written and remediation accepted.
- *evidence:* Patch manifest for remediation has changed_files=[], created_files=[], deleted_files=[], status idempotent_noop; this supports revalidation/no-op more than a substantive remediation patch.
- *source:* patch manifest lines 41-44 and 113-117
- *verdict:* weak_evidence

**F3 · `F3` · low · adversarial review**

- *claim:* Every discovered backtest run has adversarial-review.md with required markers.
- *evidence:* Glob found six adversarial-review.md and six report.json files; Grep found TASK-TDL-140/header/Residual Gaps/fail_closed_behavior markers in all six.
- *source:* Glob/Grep results
- *verdict:* supported_with_caveat

**F4 · `F4` · low · adversarial review**

- *claim:* Readiness artifacts fail closed and block promotion/live trading/high-probability claims.
- *evidence:* paper-trading-readiness.md and paper-readiness.json contain paper_trading_ready=false, promotion_eligible=false, live_trading_enabled=false, high_probability_claims_allowed=false plus failed gates and residual gaps.
- *source:* readiness markdown lines 5-16; JSON lines 76-80 and 172-177
- *verdict:* supported

**F5 · `F5` · low · adversarial review**

- *claim:* Artifact validation passed across all six backtest reviews.
- *evidence:* Validation script checks only marker strings and JSON key presence; it would not catch misleading risk analysis if markers exist.
- *source:* remediation JSON embedded validation command lines 135-139
- *verdict:* weak_evidence

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
