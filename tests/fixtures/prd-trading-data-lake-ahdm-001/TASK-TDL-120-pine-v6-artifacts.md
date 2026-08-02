# TASK-TDL-120 — Pine v6 Artifacts

```yaml
task_id: TASK-TDL-120
prd: PRD-TRADING-DATA-LAKE-AHDM-001
domain: TDL-AHDM
title: Pine v6 Artifacts
workstream: W4 AHDM Strategy
complexity: large
status: blocked
depends_on: ['TASK-TDL-110']
blocks: ['TASK-TDL-130', 'TASK-TDL-140']
source_sections: ['9', '25.1', '27']
implements: [REQ-AHDM-006, REQ-AHDM-007, REQ-AHDM-022, REQ-AHDM-023]
required_env_keys: []
required_tools: [pine_analyze, pine_check, pine_compile, pine_smart_compile, pine_get_errors, pine_get_console]
deliverable_contracts:
  - kind: pine_indicator
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/pine/AHDM-v1-indicator.pine
  - kind: pine_strategy
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/pine/AHDM-v1-strategy.pine
  - kind: pine_compile_report
    artifact_path: .archon/trading-lab/strategies/AHDM-v1/pine/compile-report.json
```

## Purpose

Generate Pine v6 indicator and strategy artifacts from the shared AHDM-v1 rule manifest.

## Scope

### In

- Pine indicator showing bias, confidence, entry zone, stop, TP1/TP2/TP3, no-trade state, sizing hint at `.archon/trading-lab/strategies/AHDM-v1/pine/AHDM-v1-indicator.pine`.
- Pine strategy encoding same rules for chart-native validation at `.archon/trading-lab/strategies/AHDM-v1/pine/AHDM-v1-strategy.pine`.
- Compile/check report at `.archon/trading-lab/strategies/AHDM-v1/pine/compile-report.json`, populated from **actual invocations of the `tradingview` MCP tools** (see "Pine Tooling Invocation (MCP)" below). Determine each tool's availability by invoking it — never by a `PATH`/`which` probe. Only if an MCP invocation itself returns a failure do you record that captured error as an explicit residual gap and fail closed on Pine promotion.
- Rule manifest shared with Archon-native implementation.

### Out

- Treating Pine Strategy Tester as promotion evidence.
- Divergent Pine-only rules.

## Pine Tooling Invocation (MCP)

The `required_tools` — `pine_analyze`, `pine_check`, `pine_compile`, `pine_smart_compile`, `pine_get_errors`, `pine_get_console` — are **MCP tools exposed by the project's `tradingview` MCP server** (configured in `.mcp.json`; `pine_compile`/`pine_smart_compile` drive TradingView Desktop over CDP). They are **not** shell/PATH executables.

- **Invoke them as MCP tools**, by their qualified names: `mcp__tradingview__pine_compile`, `mcp__tradingview__pine_smart_compile`, `mcp__tradingview__pine_check`, `mcp__tradingview__pine_analyze`, `mcp__tradingview__pine_get_errors`, `mcp__tradingview__pine_get_console`.
- **Do NOT** decide availability with `which`, `command -v`, `type`, `shutil.which`, or any `$PATH` probe. These tools are not on `PATH`; such a probe always reports them missing and is **not valid evidence** of unavailability.
- **Availability is the outcome of an actual invocation.** A tool is available if its MCP call returns a result; it is unavailable **only** if the MCP call itself returns a captured error/failure.
- `pine_compile`/`pine_smart_compile` compile the artifact against a **live TradingView chart via CDP**. If that backend is unreachable, the MCP call fails — capture that returned failure verbatim as the residual-gap evidence.
- Every `compile-report.json` tooling result and every fail-closed residual gap MUST cite the **captured MCP invocation** (the call and its returned result or error). A residual gap backed by a `PATH`/`which` probe, a source-file string mention, or a bare "unavailable" assertion is **invalid** and does not satisfy this task.

## Files Expected to Change

- Existing files only unless implementation requires a new module and user approval is obtained.
- Likely anchors: `crates/archon-trading/src/data_lake.rs`, `crates/archon-trading/src/data_store.rs`, `src/command/trading_data.rs`, `src/cli_args/trading_market_actions.rs`, `src/cli_args/tests.rs`, relevant dispatch files, and command-specific tests.

## Files Forbidden to Change

- Unrelated crates and command surfaces.
- Repository-root scratch files.
- Secrets, credentials, or local provider tokens.

## Acceptance Criteria

- AC-AHDM-002 satisfied.
- Pine artifacts are stored under `.archon/trading-lab/strategies/AHDM-v1/pine/` as exact files `AHDM-v1-indicator.pine`, `AHDM-v1-strategy.pine`, and `compile-report.json`.
- Pine and native rules are traceable through shared manifest.
- Pine results are marked exploratory only.
- `compile-report.json` tooling results record **real `mcp__tradingview__*` invocations** — each entry cites the MCP call and its returned result or error.
- A fail-closed residual gap for Pine compile/check tooling is valid **only** when backed by a captured failure from an actual `mcp__tradingview__pine_compile`/`pine_smart_compile` invocation — never a `PATH`/`which`/availability probe, a source-file mention, or a bare assertion. Such a gap cannot satisfy Pine readiness or promotion evidence.

## Focused Tests

- Pine artifact generation test.
- Structural checks for required plots/outputs.
- Real MCP invocation of `pine_analyze`/`pine_check`/`pine_compile`/`pine_smart_compile` with captured results. A `PATH`/`which` availability check is not acceptable evidence of use or of unavailability.

## Adversarial Review Notes

- Verify the task does not weaken native-candle enforcement.
- Verify unavailable provider states are honest and actionable.
- Verify compile/check evidence comes from actual `mcp__tradingview__*` invocations (captured call + returned result/error), not `PATH`/`which` probes, source-file string mentions, or bare "unavailable" assertions.
- Verify residual gaps fail closed.
- Verify no diagnostic artifact can satisfy a production promotion gate.

<!-- PRIOR-RUN-FINDINGS:BEGIN -->

### Prior run `wf-ee4a92fc` (2026-07-28) — outcome and unremediated findings

**Prior outcome:** `accepted`

> Verified TASK-TDL-120 Pine v6 artifacts are present under the project artifact root, Pine source passes TradingView MCP static analysis and server-side check, and chart-editor MCP compile paths fail closed with captured 'Could not open Pine Editor' errors. No files were changed.

**Unremediated findings against this task (5, 3 at blocking/high/medium).**

Raised by the prior run's adversarial and coverage reviews and never acted on — the review
primitive failed to stamp a task id, so every finding was classified unassigned and returned
untouched. Reproduced verbatim. They are the reviewers' claims, not established fact:
fix what is real, and refute what is wrong with specific evidence rather than editing around it.

**F1 · `F3` · blocking · adversarial review**

- *claim:* Pine artifacts verified but fail closed for promotion.
- *bounded evidence:* compile-report.json and accepted verification both record Could not open Pine Editor for chart/Pine Editor MCP calls.
- *verdict:* supports exploratory-only status; cannot support Pine readiness or promotion.

**F2 · `F1` · high · adversarial review**

- *claim:* Remediation evidence says compile-report.json was updated with call_id remediate-task-tdl-120-2-80-0 and checked_at 2026-07-29T06:42:05Z.
- *bounded evidence:* Patch manifest for remediate-task-tdl-120-2-80-0 records changed_files=[], created_files=[], deleted_files=[], status.idempotent_noop; patch file is zero lines.
- *verdict:* falsifies claimed in-branch update provenance, though current file content matches the claimed call_id.

**F3 · `F2` · medium · adversarial review**

- *claim:* TASK-TDL-120 accepted/completed by workflow.
- *bounded evidence:* Task spec YAML line 10 has status: blocked.
- *verdict:* workflow acceptance does not prove task metadata was advanced from blocked.

**F4 · `F4` · low · adversarial review**

- *claim:* Current indicator artifact is syntactically clean under Pine static analysis.
- *bounded evidence:* Fresh mcp__tradingview__pine_analyze on current indicator returned success=true, issue_count=0.
- *verdict:* not falsified for indicator static analysis.

**F5 · `F5` · low · adversarial review**

- *claim:* Required artifact files exist under project artifact root.
- *bounded evidence:* Glob found compile-report.json, AHDM-v1-strategy.pine, and AHDM-v1-indicator.pine.
- *verdict:* not falsified.

**Cross-task findings naming this task (3).**

Authored by the review reduce stage, which sees all tasks at once. No single task owns these;
they are context. Address the part that lies inside your declared scope and say plainly what
belongs elsewhere — do not attempt the whole pattern from here.

**CROSS-TASK — F1 · `REDUCE-XTASK-001` · high · adversarial review**

- *summary:* Multiple tasks are treated as accepted by workflow evidence while their task source metadata remains blocked.
- *evidence:* TDL020-ADV-003: TASK-TDL-020 task file lines 3-11 status: blocked while accepted verifier artifact status: accepted.; F-TDL030-004: Task contract YAML line 10 says status: blocked while verification branch result status is accepted with residual_gaps=[].; F-TDL110-003: TASK-TDL-110 remains status: blocked with depends_on TASK-TDL-100 while workflow acceptance treats it as accepted/noop.; TDL120 F2: Task spec YAML line 10 has status: blocked while TASK-TDL-120 accepted/completed by workflow.
- *impact:* Workflow consumers may derive conflicting execution order and readiness decisions depending on whether they trust task files or verifier artifacts.
- *status:* open
- *recommended action:* Define a single authoritative lifecycle source or update task metadata atomically as part of verified acceptance.

**CROSS-TASK — F2 · `REDUCE-XTASK-002` · high · adversarial review**

- *summary:* Several accepted/remediated results rely on no-op or empty patch provenance while claiming generated or updated deliverables.
- *evidence:* Initial audit F2: recorded generation command contains only a comment and patch file is zero-line/empty.; F-TDL030-005: implementation manifest is idempotent_noop with changed_files=[], created_files=[], deleted_files=[] and verify_command=null.; TDL120 F1: remediation evidence says compile-report.json was updated, but patch manifest has changed_files=[], created_files=[], deleted_files=[], status.idempotent_noop and patch file is zero lines.; Later Pine F2: fresh remediation accepted, but remediation patch manifest has changed_files=[], created_files=[], deleted_files=[] and status idempotent_noop.
- *impact:* Accepted artifacts may reflect pre-existing state, out-of-band writes, or unverifiable regeneration rather than the claimed in-branch implementation/remediation.
- *status:* open
- *recommended action:* Require each artifact-producing stage to record a reproducible command, content hash before/after, and non-empty artifact diff when claiming an update.

**CROSS-TASK — F3 · `REDUCE-XTASK-003` · high · adversarial review**

- *summary:* Accepted verification repeatedly reports residual_gaps=[] or satisfied acceptance while map findings document fail-closed residual gaps or unavailable required tooling.
- *evidence:* Initial F3: verification result has residual_gaps: [] while implementation result and artifact record four fail-closed residual gaps.; F-TDL030-004: verification branch accepted with residual_gaps=[] despite blocked task metadata and capability registry contradictions.; TDL120 F3: compile-report.json and accepted verification record Pine Editor MCP calls could not open, supporting exploratory-only status and blocking promotion.; Later Pine F1: required TradingView/Pine tools list has 7 entries, but four required tools failed due Pine Editor unavailable while acceptance criteria were claimed satisfied.
- *impact:* Downstream readiness decisions can overtrust accepted status even where evidence says the deliverable must remain exploratory, degraded, or blocked.
- *status:* open
- *recommended action:* Make fail-closed residual gaps first-class acceptance blockers or explicitly downgrade task status to needs_review/partial when required tools cannot run.

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
