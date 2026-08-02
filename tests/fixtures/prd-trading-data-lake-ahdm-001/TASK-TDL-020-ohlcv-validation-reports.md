# TASK-TDL-020 — OHLCV Validation Reports + Native Gates

```yaml
task_id: TASK-TDL-020
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: OHLCV Validation Reports + Native Gates
workstream: W1 Storage + Validation
complexity: large
status: blocked
depends_on: ['TASK-TDL-010']
blocks: ['TASK-TDL-030', 'TASK-TDL-090']
source_sections: ['8.2', '8.3', '8.6', '20', '21']
implements: [REQ-DL-010, REQ-DL-011, REQ-DL-012, REQ-DL-013, REQ-DL-020, REQ-DL-021, REQ-DL-022, REQ-DL-023, REQ-DL-050, REQ-DL-051, REQ-DL-052, REQ-DL-053, REQ-DL-054, REQ-DL-055, REQ-DL-056, REQ-DL-090, REQ-DL-091, REQ-DL-092, REQ-DL-093, REQ-DL-094, REQ-DL-095, REQ-DL-096, REQ-DL-097, REQ-DL-098, REQ-DL-100, REQ-DL-101, REQ-DL-102, REQ-DL-103]
required_env_keys: []
required_tools: []
deliverable_contracts:
  - kind: dataset_validation_report
    artifact_path: .archon/trading-lab/data/datasets/<dataset-id>/<version>/validation.json
    registry_path: .archon/trading-lab/data/registry.json
    registry_records_field: datasets
    instance_artifact_field: validation_path
    validation_status_field: status
    validation_checks_field: checks
    validation_check_status_field: status
    validation_failed_values: [failed]
    validation_passed_values: [passed]
```

## Purpose

Add normalized OHLCV validation reports and native interval production gates.

## Scope

### In

- `validation.json` generation.
- OHLCV JSONL validation: monotonic timestamps, duplicates, OHLC sanity, volume, gaps, metadata completeness.
- Production eligibility logic from native interval and validation status.
- CLI/TUI `data validate` command with exact dataset/action failure details and non-zero exit on failed validation unless diagnostic mode is explicit.

- Clap/CLI argument definitions and parse tests for every owned command in `src/cli_args/trading_market_actions.rs` and `src/cli_args/tests.rs`.
- Discover and update the real TUI slash-command registry/parser for every owned `/trading data ...` alias; do not assume `crates/archon-tui/src/trading/mod.rs` is the router.

### Out

- Provider-specific fetch logic.
- Backtest integration beyond reusable gate functions.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- AC-DL-005 and AC-DL-006 are covered at validation layer.
- Any validation error makes production eligibility fail.
- Missing `native_interval` or `production_eligible` means false.
- Validation report schema matches PRD §21.
- Clap parsing and command dispatch exist for every owned CLI command, with matching TUI slash alias routing where required.

## Focused Tests

- Invalid timestamp fixture fails.
- Duplicate timestamp fixture fails.
- Bad OHLC fixture fails.
- Missing metadata fixture fails.
- Missing/negative volume fixture fails where expected.
- CLI parse and TUI slash-alias routing tests for every owned command.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `failed` — but for an INFRASTRUCTURE reason, not a judgement on the work:
> fanout branch 'verification-wave-verify-task-tdl-020-13-0' produced invalid structured output after repair

The verifier never rendered an opinion on this task. Treat the prior run as giving you no
information about correctness here — neither confidence nor suspicion.

**Unremediated findings against this task (4, 4 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `TDL020-ADV-001` · high · adversarial review**

- *claim falsified:* OHLCV validation correctly enforces monotonic timestamps.
- *finding:* Timestamp monotonicity is checked by lexicographic string comparison, not parsed RFC3339 instants.
- *evidence:* validation.rs:156-164 compares bar.timestamp.as_str() < previous; validation.rs:167-184 accepts RFC3339 timestamps with timezone offsets
- *impact:* A sequence such as 2026-01-01T00:30:00Z followed by 2026-01-01T01:00:00+01:00 can sort lexically ascending while moving backward in real time.
- *status:* open
- *recommended check:* Add a mixed-offset RFC3339 test and compare parsed DateTime instants for monotonicity.

**F2 · `TDL020-ADV-002` · medium · adversarial review**

- *claim falsified:* TUI slash alias routing is verified for required data validate commands.
- *finding:* Inspected slash routing parser test covers list and export only, not validate or validate-ohlcv.
- *evidence:* src/cli_args/tests.rs:456-463 parses list/export mirrors only; Task requires CLI/TUI data validate routing
- *impact:* Accepted verification overstates direct test coverage for the owned validate slash alias.
- *status:* open
- *recommended check:* Add slash-shaped parser/routing assertions for /trading data validate and /trading data validate-ohlcv.

**F3 · `TDL020-ADV-003` · medium · adversarial review**

- *claim falsified:* Workflow acceptance is consistent with task state.
- *finding:* Task YAML still says status: blocked while verifier artifact says accepted.
- *evidence:* TASK-TDL-020 task file lines 3-11 status: blocked; Accepted verifier artifact status: accepted
- *impact:* Downstream workflow consumers may treat the task as blocked despite accepted verification.
- *status:* open
- *recommended check:* Reconcile task metadata or document why workflow acceptance does not update task status.

**F4 · `TDL020-ADV-004` · medium · adversarial review**

- *claim falsified:* Focused invalid fixture validation is fully end-to-end.
- *finding:* store_ohlcv rejects invalid bars before artifact write, while invalid duplicate/OHLC/volume focused tests inspect validation_report directly.
- *evidence:* data_store.rs:165 validates bars before write_dataset; data_store_ahdm_tests.rs:196-287 calls validation_report directly for duplicate/bad OHLC/native invariant cases
- *impact:* CLI data validate exact failure-detail behavior for already persisted invalid JSONL is less directly proven than verifier summary implies.
- *status:* open
- *recommended check:* Add an end-to-end validation command test against a deliberately invalid persisted artifact path, or clarify that ingest-time rejection satisfies the fixture requirement.

**Cross-task findings naming this task (2).**

Authored by the review reduce stage, which sees all tasks at once. No single task owns these;
they are context. Address the part that lies inside your declared scope and say plainly what
belongs elsewhere — do not attempt the whole pattern from here.

**CROSS-TASK — F1 · `REDUCE-XTASK-001` · high · adversarial review**

- *summary:* Multiple tasks are treated as accepted by workflow evidence while their task source metadata remains blocked.
- *evidence:* TDL020-ADV-003: TASK-TDL-020 task file lines 3-11 status: blocked while accepted verifier artifact status: accepted.; F-TDL030-004: Task contract YAML line 10 says status: blocked while verification branch result status is accepted with residual_gaps=[].; F-TDL110-003: TASK-TDL-110 remains status: blocked with depends_on TASK-TDL-100 while workflow acceptance treats it as accepted/noop.; TDL120 F2: Task spec YAML line 10 has status: blocked while TASK-TDL-120 accepted/completed by workflow.
- *impact:* Workflow consumers may derive conflicting execution order and readiness decisions depending on whether they trust task files or verifier artifacts.
- *status:* open
- *recommended action:* Define a single authoritative lifecycle source or update task metadata atomically as part of verified acceptance.

**CROSS-TASK — F2 · `REDUCE-XTASK-004` · medium · adversarial review**

- *summary:* Multiple task verifications overstate test coverage by testing weaker paths than the acceptance criteria require.
- *evidence:* TDL020-ADV-002: slash routing parser test covers list/export only, not required validate or validate-ohlcv.; TDL020-ADV-004: invalid duplicate/OHLC/volume focused tests inspect validation_report directly rather than end-to-end CLI validation of persisted invalid JSONL.; TDL041 F2: focused snapshot tests use ARCHON_TRADINGVIEW_SNAPSHOT_FIXTURE and do not verify live provider behavior.; F-TDL110-004: focused test suite validates non-AHDM fallback instead of rejecting it.; Later Pine F5: validation script checks marker strings and JSON key presence only, not semantic risk-analysis correctness.
- *impact:* Tests can pass while business-critical routing, live data provenance, AHDM semantics, or review quality are broken.
- *status:* open
- *recommended action:* Add negative and end-to-end tests that fail when the asserted acceptance semantics are weakened.

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
