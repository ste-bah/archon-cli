# TASK-TDL-042 — TradingView CLI + TUI Surface

```yaml
task_id: TASK-TDL-042
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: TradingView CLI + TUI Surface
workstream: W2 Providers + Coverage
complexity: medium
status: pending
depends_on: ['TASK-TDL-040']
blocks: ['TASK-TDL-080']
source_sections: ['7', '8.4', '25.1']
implements: [REQ-DL-060, REQ-DL-120, REQ-DL-121, REQ-DL-122, REQ-DL-123, REQ-DL-124]
required_env_keys: []
required_tools: []
deliverable_contracts: []
```

## Purpose

Own the command surface for the TradingView provider: clap argument definitions, parse tests, and TUI slash-alias routing. Split out of TASK-TDL-040 — interface work with no provider I/O, which was competing for the same remediation budget as live ingest.

## Scope

### In

- Full PRD fetch command signature: `archon trading data fetch-native --provider <PROVIDER> --symbol <SYMBOL> --timeframe <TF> --start <RFC3339|YYYY-MM-DD> --end <RFC3339|YYYY-MM-DD> --dataset-id <ID> [--target <PROJECT>]`.
- Clap/CLI argument definitions and parse tests for every owned command in `src/cli_args/trading_market_actions.rs` and `src/cli_args/tests.rs`.
- Discover and update the real TUI slash-command registry/parser for every owned `/trading data ...` alias; do not assume `crates/archon-tui/src/trading/mod.rs` is the router — find the actual router and prove the routing.
- `data snapshot --provider tradingview` argument surface.

### Out

- Provider I/O, MCP invocation, paging, registry writes (TASK-TDL-040 owns those).
- Snapshot capture and staleness semantics (TASK-TDL-041 owns those).

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, the real TUI slash-command registry/parser (discover it — do not assume), relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Provider implementation modules owned by TASK-TDL-040/041.
- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- Clap parsing and command dispatch exist for every owned CLI command.
- CLI parse tests require all mandatory `fetch-native` flags: `--provider`, `--symbol`, `--timeframe`, `--start`, `--end`, and `--dataset-id`; `--target` remains optional.
- Every owned `/trading data ...` TUI alias routes to its command, proven against the REAL registry/parser rather than an assumed one — name the router file you found in the evidence.
- Alias routing is proven by an executed test, not by inspection alone.

## Focused Tests

- CLI parse tests for every owned command, including missing-mandatory-flag rejection.
- TUI `/trading data ...` alias routing test for every owned alias.

## Adversarial Review Notes

- Verify alias routing was proven against the real registry, not an assumed module path.
- Verify parse tests actually reject missing mandatory flags rather than only accepting valid input.
- Verify residual gaps fail closed.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `noop`

> Read-only continuity/review stage completed. No actionable implementation request was present in the supplied context summary, and no files were changed. Inspected existing trading data CLI/TUI surfaces and verified the focused mandatory fetch-native parser test passes under the correct package.

**Unremediated findings against this task (3, 3 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `TASK-TDL-042-CANONICAL-COVERAGE-MISSING` · high · coverage review**

- *finding:* Implementation task_coverage is for context-continuity-summary, not TASK-TDL-042, and the implementation branch reported no actionable task and no file changes.
- *evidence:* /Volumes/Externalwork/archon-cli/project-1/.archon/workflows/wf-ee4a92fc-aaee-40ca-ab65-1587f7df6b0f/v2/results/implement-task-tdl-042-34-6354b299536285667378d4742e64c2d7baf4cce3519784fa5bffaec52bd52400.json:95-115

**F2 · `TASK-TDL-042-TUI-ALIAS-ROUTING-NOT-PROVEN` · high · coverage review**

- *finding:* Verification recorded cargo test -p archon-tui filter_trading_data_aliases timed out after 120 seconds; no successful executed TUI alias-routing test is present.
- *evidence:* /Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001/TASK-TDL-042-tradingview-cli-tui-surface.md:53-59; /Volumes/Externalwork/archon-cli/project-1/.archon/workflows/wf-ee4a92fc-aaee-40ca-ab65-1587f7df6b0f/v2/results/verification-wave-verify-task-tdl-042-35-374a76d09efbfd253c67b58c527a0514a558338436aefdea4660ca6edb95f8f0.json:113-118; /Volumes/Externalwork/archon-cli/project-1/.archon/workflows/wf-ee4a92fc-aaee-40ca-ab65-1587f7df6b0f/v2/results/verification-wave-verify-task-tdl-042-35-374a76d09efbfd253c67b58c527a0514a558338436aefdea4660ca6edb95f8f0.json:196-199

**F3 · `TASK-TDL-042-ALL-OWNED-CLI-DISPATCH-NOT-COVERED` · medium · coverage review**

- *finding:* Accepted verification proves one fetch-native mandatory-flag parser test passed, but does not show executed dispatch coverage for every owned CLI command or snapshot command surface.
- *evidence:* /Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001/TASK-TDL-042-tradingview-cli-tui-surface.md:27-30; /Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001/TASK-TDL-042-tradingview-cli-tui-surface.md:51-52; /Volumes/Externalwork/archon-cli/project-1/.archon/workflows/wf-ee4a92fc-aaee-40ca-ab65-1587f7df6b0f/v2/results/verification-wave-verify-task-tdl-042-35-374a76d09efbfd253c67b58c527a0514a558338436aefdea4660ca6edb95f8f0.json:51-54; /Volumes/Externalwork/archon-cli/project-1/.archon/workflows/wf-ee4a92fc-aaee-40ca-ab65-1587f7df6b0f/v2/results/verification-wave-verify-task-tdl-042-35-374a76d09efbfd253c67b58c527a0514a558338436aefdea4660ca6edb95f8f0.json:173-187

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
