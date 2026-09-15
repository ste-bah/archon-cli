export const meta = {
  name: 'trading-data-lake-ahdm-v1',
  description: 'Implement the PRD-TRADING-DATA-LAKE-AHDM-001 task set (data-lake crates + cited AHDM-v1 evidence, spec, Pine, native backtest, readiness) and verify each declared focused test.',
  phases: [
    { title: 'Task Work', detail: 'Implement every canonical task in host-computed dependency waves; adversarially verify each; remediate with a bounded progress budget.' },
    { title: 'Review', detail: 'Critic-tier adversarial review and source-coverage audit over every canonical task exactly once, then bounded review remediation.' },
  ],
}

// ---- helpers -------------------------------------------------------------
function isAccepted(env) {
  if (!env) return false
  if (env.status === 'noop') return true
  if (env.status !== 'accepted') return false
  const files = Array.isArray(env.files_changed) ? env.files_changed.length : 0
  const cmds = Array.isArray(env.commands_run) ? env.commands_run.length : 0
  return files > 0 || cmds > 0
}

function remediationEvidence(env) {
  if (!env) return 'no envelope'
  let outputSummaryBudget = 4000
  return JSON.stringify(env, (key, value) => {
    if (key !== 'output_summary' || typeof value !== 'string') return value
    const kept = value.slice(0, outputSummaryBudget)
    outputSummaryBudget -= kept.length
    return kept.length === value.length ? kept : `${kept}\n[output_summary truncated]`
  })
}

function summarize(env) {
  if (!env) return 'no envelope'
  const s = env.summary || (env.result && env.result.summary) || JSON.stringify(env).slice(0, 400)
  return String(s).slice(0, 600)
}

const budgetFactory = (typeof remediationBudget === 'function')
  ? remediationBudget
  : () => ({ shouldContinue: (attempt) => attempt < 6 })

function boundedEvidenceFor(taskId) {
  const env = finalImpl[taskId]
  if (!env) return [{ kind: 'other', summary: 'no implementation envelope retained for ' + taskId }]
  return [{
    kind: 'implementation',
    summary: String(env.summary || '').slice(0, 1200),
    files_changed: (Array.isArray(env.files_changed) ? env.files_changed : []).slice(0, 20),
    commands: (Array.isArray(env.commands_run) ? env.commands_run : []).map((c) => c && c.command).filter(Boolean).slice(0, 12),
  }]
}

// ---- task universe (ids, files, target files taken verbatim from the task index)
const TASKS_ROOT = '/Volumes/Externalwork/archon-cli/project-1/tasks/PRD-TRADING-DATA-LAKE-AHDM-001'
const tasks = [
  { id: 'TASK-DL-001', file: TASKS_ROOT + '/TASK-DL-001.md', targetFiles: ['docs/trading-data-lake-gap-audit.md'], note: 'Read-only gap audit; the ONLY file written is the dated audit report at that exact path.' },
  { id: 'TASK-DL-002', file: TASKS_ROOT + '/TASK-DL-002.md', targetFiles: ['crates/archon-trading/src/data_lake.rs', 'crates/archon-trading/src/data_lake/registry.rs', 'crates/archon-trading/src/data_lake/migration.rs', 'crates/archon-trading/tests/registry_v2.rs', 'crates/archon-trading/tests/dataset_naming.rs', 'crates/archon-trading/tests/migration_v1_to_v2.rs', 'crates/archon-trading/tests/ingest_artifacts.rs', 'src/command/trading_data.rs'], note: 'Registry artifacts are mutated only by the code paths during focused runs, never hand-edited.' },
  { id: 'TASK-DL-003', file: TASKS_ROOT + '/TASK-DL-003.md', targetFiles: ['crates/archon-trading/src/validation.rs', 'crates/archon-trading/src/validation/checks.rs', 'crates/archon-trading/src/validation/report.rs', 'crates/archon-trading/src/lib.rs', 'crates/archon-trading/src/data_lake.rs', 'crates/archon-trading/tests/validation_rules.rs', 'crates/archon-trading/tests/validation_report.rs', 'crates/archon-trading/tests/native_interval_gates.rs', 'src/command/trading_data.rs'], note: 'Only the minimal validation seam swap inside data_lake plus lib wiring; data_lake storage logic is TASK-DL-002 territory.' },
  { id: 'TASK-DL-004', file: TASKS_ROOT + '/TASK-DL-004.md', targetFiles: ['crates/archon-trading/src/providers/mod.rs', 'crates/archon-trading/src/providers/capability.rs', 'crates/archon-trading/src/providers/registry.rs', 'crates/archon-trading/src/lib.rs', 'crates/archon-trading/tests/provider_capability_types.rs', 'crates/archon-trading/tests/capability_engine.rs', 'crates/archon-trading/tests/capability_unavailable.rs', 'src/command/trading_data.rs'], note: 'Capability probing must be honest and fail closed; provider-capabilities.json is code-path-written only. Live TradingView MCP calls are expected.' },
  { id: 'TASK-DL-010', file: TASKS_ROOT + '/TASK-DL-010.md', targetFiles: ['crates/archon-trading/src/backtest_gates.rs', 'crates/archon-trading/src/lib.rs', 'crates/archon-trading/tests/backtest_gate_refusals.rs', 'crates/archon-trading/tests/backtest_diagnostic_override.rs', 'crates/archon-trading/tests/backtest_report_replay.rs', 'src/command/trading_data.rs'], note: 'Runs artifacts are code-path-written only; only the minimal backtest-entry seam in the command surface if needed.' },
  { id: 'TASK-DL-005', file: TASKS_ROOT + '/TASK-DL-005.md', targetFiles: ['crates/archon-trading/src/providers/tradingview.rs', 'crates/archon-trading/src/providers/mod.rs', 'crates/archon-trading/tests/tradingview_adapter_mapping.rs', 'crates/archon-trading/tests/tradingview_adapter_unavailable.rs', 'crates/archon-trading/tests/tradingview_ingest_artifacts.rs'], note: 'Only the one registration line in providers/mod.rs; dataset/snapshot writes happen via the adapter code path in focused runs, never hand-edited. Live MCP calls required.' },
  { id: 'TASK-DL-006', file: TASKS_ROOT + '/TASK-DL-006.md', targetFiles: ['crates/archon-trading/src/providers/openbb_polygon.rs', 'crates/archon-trading/src/providers/mod.rs', 'crates/archon-trading/tests/openbb_polygon_adapter_mapping.rs', 'crates/archon-trading/tests/openbb_polygon_adapter_unavailable.rs', 'crates/archon-trading/tests/openbb_polygon_ingest_artifacts.rs'], note: 'Missing POLYGON_API_KEY must fail closed naming the credential; never leak credentials into artifacts.' },
  { id: 'TASK-DL-007', file: TASKS_ROOT + '/TASK-DL-007.md', targetFiles: ['crates/archon-trading/src/providers/stooq.rs', 'crates/archon-trading/src/providers/mod.rs', 'crates/archon-trading/tests/stooq_adapter_mapping.rs', 'crates/archon-trading/tests/stooq_adapter_unavailable.rs', 'crates/archon-trading/tests/stooq_ingest_artifacts.rs'], note: 'Exact-native ingest or honest unavailable lane classified per the task file; fail closed otherwise.' },
  { id: 'TASK-DL-008', file: TASKS_ROOT + '/TASK-DL-008.md', targetFiles: ['crates/archon-trading/src/providers/yfinance.rs', 'crates/archon-trading/src/providers/mod.rs', 'crates/archon-trading/tests/yfinance_adapter_mapping.rs', 'crates/archon-trading/tests/yfinance_adapter_unavailable.rs', 'crates/archon-trading/tests/yfinance_ingest_artifacts.rs'], note: 'Degraded fallback provenance is mandatory (raw/provider-notes.md); failure lanes fail closed.' },
  { id: 'TASK-DL-009', file: TASKS_ROOT + '/TASK-DL-009.md', targetFiles: ['crates/archon-trading/src/coverage.rs', 'crates/archon-trading/src/lib.rs', 'crates/archon-trading/tests/coverage_matrix_generation.rs', 'crates/archon-trading/tests/coverage_selection_freshness.rs', 'crates/archon-trading/tests/data_commands_surface.rs', 'crates/archon-tui/src/commands.rs', 'src/command/trading_data.rs'], note: 'Coverage artifacts are written only by the coverage code path during focused runs; never hand-edited. 30-cell matrix, fail-closed fallback_reason on unavailable cells.' },
  { id: 'TASK-AHDM-001', file: TASKS_ROOT + '/TASK-AHDM-001.md', targetFiles: ['.archon/trading-lab/strategies/AHDM-v1/evidence/kb-rule-inventory.md', '.archon/trading-lab/strategies/AHDM-v1/evidence/citations.json'], note: 'Do NOT create pine/, backtests/ or readiness/ subtrees or strategy-spec.json; the coverage gate (.archon/trading-lab/data/coverage/latest.json, 30 cells) must pass before the inventory ships; every rule needs a real memory_recall query recorded.' },
  { id: 'TASK-AHDM-002', file: TASKS_ROOT + '/TASK-AHDM-002.md', targetFiles: ['.archon/trading-lab/strategies/AHDM-v1/strategy-spec.json'], note: 'Consume the TASK-AHDM-001 evidence read-only; bind datasets to the registry/coverage or record pending-ingest with residual gaps; thresholds 0.70/0.55; no high-probability claims.' },
  { id: 'TASK-AHDM-003', file: TASKS_ROOT + '/TASK-AHDM-003.md', targetFiles: ['.archon/trading-lab/strategies/AHDM-v1/pine/AHDM-v1-indicator.pine', '.archon/trading-lab/strategies/AHDM-v1/pine/AHDM-v1-strategy.pine', '.archon/trading-lab/strategies/AHDM-v1/pine/compile-report.json'], note: 'Pine v6, //@version=6 on line 1, exploratory markers, shared rule_id comments matching the spec manifest; validate with mcp__tradingview__pine_check and record both declared MCP calls in compile-report.json mcp_calls; never modify the consumed spec/evidence.' },
  { id: 'TASK-AHDM-004', file: TASKS_ROOT + '/TASK-AHDM-004.md', targetFiles: ['crates/archon-trading/src/ahdm_backtest.rs', 'crates/archon-trading/src/lib.rs', 'crates/archon-trading/tests/ahdm_backtest.rs'], note: 'Native backtest engine executing the StrategySpec on gated datasets; run reports and blocked-state.json are produced by the engine code path only; a spec defect is reported upstream, never patched here.' },
  { id: 'TASK-AHDM-005', file: TASKS_ROOT + '/TASK-AHDM-005.md', targetFiles: ['.archon/trading-lab/strategies/AHDM-v1/readiness/paper-trading-readiness.md'], note: 'Adversarial review: write backtests/run-*/adversarial-review.md ONLY inside run directories that already exist from TASK-AHDM-004; never create or edit config.json/report.json/trades.jsonl/equity_curve.jsonl or any crates/ or pine/ or data/ file; explicit readiness granted/refused verdict, paper-only statement, fail-closed semantics, recorded memory_recall queries.' },
]

// Static literal id list — used by the mandatory reviews so each review's map
// coverage over every canonical task id is statically provable to the host
// (one critic reviewer per task id, exactly once, for BOTH reviews).
const ALL_TASK_IDS = [
  'TASK-DL-001',
  'TASK-DL-002',
  'TASK-DL-003',
  'TASK-DL-004',
  'TASK-DL-005',
  'TASK-DL-006',
  'TASK-DL-007',
  'TASK-DL-008',
  'TASK-DL-009',
  'TASK-DL-010',
  'TASK-AHDM-001',
  'TASK-AHDM-002',
  'TASK-AHDM-003',
  'TASK-AHDM-004',
  'TASK-AHDM-005',
]

// Waves copied exactly from the host EXECUTION WAVES — wave 9 batches AHDM-003+004,
// the same-wave DL-004/DL-010 and DL-005..008 pairs stay separate single-call groups
// per the execution batches JSON (separate groups at one wave serialize for write conflicts).
const waves = [
  ['TASK-DL-001'],
  ['TASK-DL-002'],
  ['TASK-DL-003'],
  ['TASK-DL-004'],
  ['TASK-DL-010'],
  ['TASK-DL-005'],
  ['TASK-DL-006'],
  ['TASK-DL-007'],
  ['TASK-DL-008'],
  ['TASK-DL-009'],
  ['TASK-AHDM-001'],
  ['TASK-AHDM-002'],
  ['TASK-AHDM-003', 'TASK-AHDM-004'],
  ['TASK-AHDM-005'],
]

// Declared focused tests, verbatim from each task file (only the executable command
// lines the task authors ran; no invented or widened commands).
const focusedByTask = {
  'TASK-DL-001': [
    "test -s \"$R\"",
    "test \"$(wc -l < \"$R\")\" -lt 500",
    "for a in data_lake.rs data_store.rs trading_data.rs; do grep -q \"$a\" \"$R\" || exit 1; done",
    "grep -q 'archon-trading-data-registry-v' \"$R\"",
    "grep -q 'data-lake-gap-audit' \"$R\"",
    "grep -q 'fail_closed_behavior' \"$R\" && grep -q 'fail_closed_check' \"$R\"",
    "grep -q 'provider-capabilities' \"$R\" && grep -q 'backup' \"$R\"",
    "grep -Eq 'REQ-(DL|AHDM|BT)-[0-9]+' \"$R\"",
    "grep -Eq 'TASK-(DL|AHDM)-[0-9]+|GAP-(DL|AHDM)-[0-9]+' \"$R\"",
    "grep -Eq '[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}' \"$R\"",
    "grep -Eq 'metadata\\.json|REQ-DL-010' \"$R\"",
    "grep -Eni 'should work|probably available|\\bTBD\\b|some provider|best effort' \"$R\"; test $? -eq 1",
    "grep -q 'archon-trading-data-registry-v' .archon/trading-lab/data/registry.json",
  ],
  'TASK-DL-002': [
    "cargo nextest run -p archon-trading --test registry_v2",
    "cargo nextest run -p archon-trading --test dataset_naming",
    "cargo nextest run -p archon-trading --test migration_v1_to_v2",
    "cargo nextest run -p archon-trading --test ingest_artifacts",
    "cargo clippy -p archon-trading -- -D warnings",
    "cargo build --release --bin archon",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' crates/archon-trading/src/data_lake.rs",
    "test -s crates/archon-trading/src/data_lake.rs",
    "test -s .archon/trading-lab/data/registry-migration-report.json",
    "test -d .archon/trading-lab/data/coverage",
    "grep -q 'archon-trading-data-registry' .archon/trading-lab/data/registry.json",
    "grep -Eq 'migrated|skipped|degraded|failed' .archon/trading-lab/data/registry-migration-report.json",
  ],
  'TASK-DL-003': [
    "cargo nextest run -p archon-trading --test validation_rules",
    "cargo nextest run -p archon-trading --test validation_report",
    "cargo nextest run -p archon-trading --test native_interval_gates",
    "cargo nextest run -p archon-trading --test native_interval_gates derived_is_non_production",
    "cargo clippy -p archon-trading -- -D warnings",
    "cargo build --release --bin archon",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' crates/archon-trading/src/validation.rs",
    "test -s crates/archon-trading/src/validation.rs",
  ],
  'TASK-DL-004': [
    "cargo nextest run -p archon-trading --test capability_engine",
    "cargo nextest run -p archon-trading --test capability_unavailable",
    "cargo nextest run -p archon-trading --test provider_capability_types",
    "cargo clippy -p archon-trading -- -D warnings",
    "cargo build --release --bin archon",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' crates/archon-trading/src/providers/mod.rs",
    "test -s crates/archon-trading/src/providers/mod.rs",
    "test -s .archon/trading-lab/data/provider-capabilities.json",
    "python3 -c \"import json,sys; d=json.load(open(sys.argv[1])); assert d and all(('provider' in r and 'checked_at' in r) for r in (d if isinstance(d,list) else d.get('results',[d])))\" .archon/trading-lab/data/provider-capabilities.json",
  ],
  'TASK-DL-010': [
    "cargo nextest run -p archon-trading --test backtest_gate_refusals",
    "cargo nextest run -p archon-trading --test backtest_diagnostic_override",
    "cargo nextest run -p archon-trading --test backtest_report_replay",
    "cargo check -p archon-trading --tests",
    "cargo clippy -p archon-trading -- -D warnings",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' crates/archon-trading/src/backtest_gates.rs",
    "test -s crates/archon-trading/src/backtest_gates.rs",
  ],
  'TASK-DL-005': [
    "cargo nextest run -p archon-trading --test tradingview_adapter_mapping",
    "cargo nextest run -p archon-trading --test tradingview_adapter_unavailable",
    "cargo nextest run -p archon-trading --test tradingview_ingest_artifacts",
    "cargo check -p archon-trading --tests",
    "cargo clippy -p archon-trading -- -D warnings",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' crates/archon-trading/src/providers/tradingview.rs",
    "test -s crates/archon-trading/src/providers/tradingview.rs",
  ],
  'TASK-DL-006': [
    "cargo nextest run -p archon-trading --test openbb_polygon_adapter_mapping",
    "cargo nextest run -p archon-trading --test openbb_polygon_adapter_unavailable",
    "cargo nextest run -p archon-trading --test openbb_polygon_ingest_artifacts",
    "cargo check -p archon-trading --tests",
    "cargo clippy -p archon-trading -- -D warnings",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' crates/archon-trading/src/providers/openbb_polygon.rs",
    "test -s crates/archon-trading/src/providers/openbb_polygon.rs",
  ],
  'TASK-DL-007': [
    "cargo nextest run -p archon-trading --test stooq_adapter_mapping",
    "cargo nextest run -p archon-trading --test stooq_adapter_unavailable",
    "cargo nextest run -p archon-trading --test stooq_ingest_artifacts",
    "cargo check -p archon-trading --tests",
    "cargo clippy -p archon-trading -- -D warnings",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' crates/archon-trading/src/providers/stooq.rs",
    "test -s crates/archon-trading/src/providers/stooq.rs",
  ],
  'TASK-DL-008': [
    "cargo nextest run -p archon-trading --test yfinance_adapter_mapping",
    "cargo nextest run -p archon-trading --test yfinance_adapter_unavailable",
    "cargo nextest run -p archon-trading --test yfinance_ingest_artifacts",
    "cargo check -p archon-trading --tests",
    "cargo clippy -p archon-trading -- -D warnings",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' crates/archon-trading/src/providers/yfinance.rs",
    "test -s crates/archon-trading/src/providers/yfinance.rs",
  ],
  'TASK-DL-009': [
    "cargo nextest run -p archon-trading --test coverage_matrix_generation",
    "cargo nextest run -p archon-trading --test coverage_selection_freshness",
    "cargo nextest run -p archon-trading --test data_commands_surface",
    "cargo check -p archon-trading --tests",
    "cargo clippy -p archon-trading -- -D warnings",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' crates/archon-trading/src/coverage.rs",
    "test -s crates/archon-trading/src/coverage.rs",
    "test -s .archon/trading-lab/data/coverage/latest.json",
    "test -s .archon/trading-lab/data/coverage/latest.md",
  ],
  'TASK-AHDM-001': [
    "test -s \"$INV\" && test -s \"$CIT\"",
    "awk 'END{exit (NR>500)}' \"$INV\"",
    "grep -Eqi 'TBD' \"$INV\" && { echo 'FAIL: TBD present'; exit 1; } || true",
    "grep -qi 'trading-elliott-wave' \"$INV\" || { echo 'FAIL: elliott-wave disposition missing'; exit 1; }",
    "! grep -rEqi 'should work|probably available|best effort' \"$INV\" \"$CIT\" || { echo 'FAIL: vague phrase present'; exit 1; }",
    "test ! -e .archon/trading-lab/strategies/AHDM-v1/backtests",
    "test ! -e .archon/trading-lab/strategies/AHDM-v1/pine",
    "test ! -e .archon/trading-lab/strategies/AHDM-v1/strategy-spec.json",
  ],
  'TASK-AHDM-002': [
    "test -s \"$INV\" && test -s \"$CIT\" || { echo 'FAIL: TASK-AHDM-001 evidence artifacts missing'; exit 1; }",
    "python3 -c \"import json,sys; json.load(open('$CIT'))\" || exit 1",
    "test ! -e .archon/trading-lab/strategies/AHDM-v1/backtests",
    "test ! -e .archon/trading-lab/strategies/AHDM-v1/pine",
    "test ! -e .archon/trading-lab/strategies/AHDM-v1/readiness",
  ],
  'TASK-AHDM-003': [
    "test -s \"$IND\" && test -s \"$STR\" && test -s \"$REP\" || { echo 'FAIL: pine artifacts missing'; exit 1; }",
    "test -s \"$SPEC\" || { echo 'FAIL: TASK-AHDM-002 strategy-spec.json missing'; exit 1; }",
    "head -n1 \"$IND\" | grep -qx '//@version=6' || { echo 'FAIL: indicator not //@version=6 line 1'; exit 1; }",
    "head -n1 \"$STR\" | grep -qx '//@version=6' || { echo 'FAIL: strategy not //@version=6 line 1'; exit 1; }",
    "grep -q 'indicator(' \"$IND\" || { echo 'FAIL: indicator( declaration missing'; exit 1; }",
    "grep -q 'strategy(' \"$STR\" || { echo 'FAIL: strategy( declaration missing'; exit 1; }",
    "awk 'END{exit (NR>500)}' \"$IND\" || { echo 'FAIL: indicator over 500 lines'; exit 1; }",
    "awk 'END{exit (NR>500)}' \"$STR\" || { echo 'FAIL: strategy over 500 lines'; exit 1; }",
    "test ! -e .archon/trading-lab/strategies/AHDM-v1/backtests",
    "test ! -e .archon/trading-lab/strategies/AHDM-v1/readiness",
  ],
  'TASK-AHDM-004': [
    "cargo nextest run -p archon-trading --test ahdm_backtest gate_refusals",
    "cargo nextest run -p archon-trading --test ahdm_backtest determinism",
    "cargo nextest run -p archon-trading --test ahdm_backtest run_record",
    "cargo nextest run -p archon-trading --test ahdm_backtest spec_parity",
    "cargo nextest run -p archon-trading --test ahdm_backtest thresholds",
    "cargo nextest run -p archon-trading --test ahdm_backtest diagnostic",
    "cargo nextest run -p archon-trading --test ahdm_backtest atomic_writes",
    "cargo check -p archon-trading --tests",
    "cargo clippy -p archon-trading -- -D warnings",
    "bash scripts/check-file-sizes.sh",
    "awk 'END{exit (NR>500)}' \"$ENG\" || { echo 'FAIL: ahdm_backtest.rs over 500 lines'; exit 1; }",
    "test -s \"$ENG\"",
    "test -s \"$SPEC\" || { echo 'FAIL: TASK-AHDM-002 strategy-spec.json missing'; exit 1; }",
    "test -s crates/archon-trading/src/backtest_gates.rs || { echo 'FAIL: TASK-DL-010 backtest_gates.rs missing'; exit 1; }",
  ],
  'TASK-AHDM-005': [
    "test -s \"$RD\" || { echo 'FAIL: readiness report missing'; exit 1; }",
    "awk 'END{exit (NR>500)}' \"$RD\" || { echo 'FAIL: readiness report over 500 lines'; exit 1; }",
    "grep -q 'adversarial' \"$RD\" || { echo 'FAIL: readiness does not reference adversarial review'; exit 1; }",
    "grep -q 'memory_recall' \"$RD\" || { echo 'FAIL: memory_recall query not recorded'; exit 1; }",
    "grep -q 'strategy-spec.json' \"$RD\" || { echo 'FAIL: report cites no inspected artifacts'; exit 1; }",
    "grep -qi 'paper' \"$RD\" || { echo 'FAIL: report lacks paper-only statement'; exit 1; }",
    "test -s \"$ROOT/strategy-spec.json\" || { echo 'FAIL: TASK-AHDM-002 spec missing'; exit 1; }",
    "test -s \"$ROOT/pine/AHDM-v1-indicator.pine\" || { echo 'FAIL: TASK-AHDM-003 indicator missing'; exit 1; }",
    "test -s crates/archon-trading/src/ahdm_backtest.rs || { echo 'FAIL: TASK-AHDM-004 engine missing'; exit 1; }",
  ],
}

const artifactsByTask = {
  'TASK-DL-001': ['docs/trading-data-lake-gap-audit.md'],
  'TASK-AHDM-001': ['.archon/trading-lab/strategies/AHDM-v1/evidence/kb-rule-inventory.md', '.archon/trading-lab/strategies/AHDM-v1/evidence/citations.json'],
  'TASK-AHDM-002': ['.archon/trading-lab/strategies/AHDM-v1/strategy-spec.json'],
  'TASK-AHDM-003': ['.archon/trading-lab/strategies/AHDM-v1/pine/AHDM-v1-indicator.pine', '.archon/trading-lab/strategies/AHDM-v1/pine/AHDM-v1-strategy.pine', '.archon/trading-lab/strategies/AHDM-v1/pine/compile-report.json'],
  'TASK-AHDM-005': ['.archon/trading-lab/strategies/AHDM-v1/readiness/paper-trading-readiness.md'],
}

const byId = (id) => tasks.find((t) => t.id === id)
const acceptedTaskIds = []
const blockedTasks = []
const implOf = {}
const finalImpl = {}

phase('Task Work')
log('implementing ' + tasks.length + ' canonical tasks across ' + waves.length + ' host-computed waves')

// IMPLEMENT BY WAVE — every task in one wave goes in ONE agents([...]) call.
for (const wave of waves) {
  const batch = await agents(
    wave.map((id) => ({
      prompt: `Implement ${id} per ${byId(id).file}. Goal: satisfy that task's acceptance criteria exactly — read the task file FIRST, honor its files_forbidden_to_change and its declared focused tests, and ${byId(id).note} Resolve repository paths against the repository_root in YOUR OWN stage input — the host stamps your isolated checkout there; resolve anything under .archon/ artifact trees against the project_artifact_root in YOUR OWN stage input. NEVER paste an absolute repository path into this prompt or your output. Re-inspect the current repository and artifact state BEFORE writing — if the work is genuinely already done, verify that with real evidence and return the typed no-op (status noop, idempotent_noop true, task_coverage evidence naming the files/tests that prove it); NEVER make cosmetic edits just to show work. Decide your own implementation approach, run the narrowest commands that actually prove the change IN-SESSION (each cargo test filter as its own invocation, one filter per command), fix your own command mistakes, and report files_changed and commands_run honestly. An outcome with no changed files and no commands is treated as a failure unless it is a proper typed no-op with proof.`,
      label: `implement-${id.toLowerCase()}`,
      taskIds: [id],
      targetFiles: byId(id).targetFiles,
      ...(focusedByTask[id] && focusedByTask[id].length ? { focusedTests: focusedByTask[id] } : {}),
      ...(artifactsByTask[id] ? { artifacts: artifactsByTask[id] } : {}),
    })),
    { write: true, maxParallelism: wave.length },
  )
  const branches = outcomesOf(batch)
  for (const id of wave) {
    const branch = branches.find((o) => (o.canonical_task_ids || []).includes(id))
    // A wave item with no outcome is a host-side/transport failure, not a verdict: record it so the loop below retries instead of accepting silently.
    implOf[id] = branch || { status: 'failed', summary: `no outcome returned for ${id} in its wave batch (possible transport error — retry, not a verdict)` }
  }
}

// VERIFY AND REMEDIATE PER TASK — sequential, budget follows progress, max 6 attempts.
for (const t of tasks) {
  let impl = implOf[t.id]
  finalImpl[t.id] = impl
  let check = await agent(`You did NOT implement ${t.id} — be suspicious of its self-report. Re-read ${t.file}, inspect the actual code and artifacts yourself, and run whatever tests YOU judge prove or disprove the acceptance criteria (run each cargo test filter as its own invocation). If the implementation claims a no-op, demand and check the proof that the work already exists.`, { label: `verify-${t.id.toLowerCase()}`, verify: true, taskIds: [t.id] })
  const budget = budgetFactory()
  for (let attempt = 2; attempt <= 6 && budget.shouldContinue(attempt - 1, check, impl) && (!isAccepted(impl) || !isAccepted(check)); attempt += 1) {
    const rejectedAttempt = `Implementation envelope:\n${remediationEvidence(impl)}\nVerifier envelope:\n${remediationEvidence(check)}`
    impl = await agent(`Remediate ${t.id}. The previous attempt was REJECTED. Fix exactly what these verbatim implementation and verifier envelopes name; do not re-argue them:\n${rejectedAttempt}\nOriginal goal: implement ${t.id} per ${t.file} — ${t.note} Resolve repository paths against the repository_root in YOUR OWN stage input and .archon/ artifact trees against the project_artifact_root in YOUR OWN stage input — never an absolute path written into this prompt. Prove the fix with tests you run yourself, one filter per command invocation, and report files_changed and commands_run honestly.`, { label: `remediate-${t.id.toLowerCase()}-${attempt}`, write: true, taskIds: [t.id], targetFiles: t.targetFiles })
    check = await agent(`You did NOT implement ${t.id} — be suspicious. The previous attempt was rejected with these verbatim findings:\n${rejectedAttempt}\nRe-read ${t.file}, inspect the actual code and artifacts, and run whatever tests YOU judge prove or disprove the acceptance criteria.`, { label: `verify-${t.id.toLowerCase()}-${attempt}`, verify: true, taskIds: [t.id] })
  }
  finalImpl[t.id] = impl
  if (isAccepted(impl) && isAccepted(check)) {
    acceptedTaskIds.push(t.id)
  } else {
    blockedTasks.push({ taskId: t.id, reason: summarize(check) || summarize(impl) })
  }
}

phase('Review')
log(acceptedTaskIds.length + ' accepted, ' + blockedTasks.length + ' blocked; running mandatory reviews')
// BOTH mandatory reviews fan out over the STATIC literal ALL_TASK_IDS list so that
// map coverage over every canonical task id (each accepted task exactly once) is
// statically provable to the host — not the dynamically built acceptedTaskIds array.
// The review primitives themselves keep one critic map item per id and a
// reduce_final reducer with preserveMapFindings; findings are read only through
// the primitives' returned results and handed to remediateFindings unmodified.
const adversarial_findings = await adversarialReview(ALL_TASK_IDS, { evidenceFor: boundedEvidenceFor })
const uncovered_requirements = await coverageAudit(ALL_TASK_IDS, { evidenceFor: boundedEvidenceFor })
const review_remediation = await remediateFindings([...adversarial_findings, ...uncovered_requirements], { blockedTasks, taskFileFor: (id) => (tasks.find((t) => t.id === id) || {}).file, targetFilesFor: (id) => (tasks.find((t) => t.id === id) || {}).targetFiles })

return {
  accepted: acceptedTaskIds,
  blocked: blockedTasks,
  adversarial_findings,
  uncovered_requirements,
  review_remediation,
  notes: 'Implemented the 15-task PRD-TRADING-DATA-LAKE-AHDM-001 universe in the host-computed waves with per-task adversarial verification and bounded remediation, then ran both mandatory critic reviews over the full static task-id list (each task reviewed exactly once) and bounded review remediation; every canonical task id appears exactly once across accepted+blocked.',
}