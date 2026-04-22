# CI Scripts

Lint and validation scripts invoked by `.github/workflows/*.yml` (wired in TASK-TUI-814).

| Script | Purpose | Spec |
|--------|---------|------|
| `check-tui-file-sizes.sh` | Fail if any `crates/archon-tui/src/**/*.rs` > 500 lines | TECH-TUI-OBSERVABILITY line 1252, NFR-TUI-QUAL-001 |
| `check-cycles.sh` | Fail if workspace dep graph has any SCC > 1 (circular dep) | TECH-TUI-OBSERVABILITY line 1129, NFR-TUI-MOD-002 |
| `check-duplicate-code.sh` | Fail if jscpd reports >=5% code duplication in archon-tui/src | TECH-TUI-OBSERVABILITY line 1130, NFR-TUI-MOD-003 |
| `grep-bounded-channel.sh` | Fail if any `mpsc::channel::<AgentEvent>` (bounded) exists | TECH-TUI-OBSERVABILITY line 1131 |
| `grep-await-send.sh` | Fail if any `agent_event_tx.send(...).await` exists | TECH-TUI-OBSERVABILITY line 1132 |
| `check-complexity.sh` | Fail on any function with clippy cognitive_complexity >= 10 in `crates/archon-tui` | TECH-TUI-OBSERVABILITY line 1128, AC-OBSERVABILITY-02, NFR-TUI-QUAL-002 |
| `check-coverage.sh` | Fail if `cargo llvm-cov --package archon-tui` line coverage < `$COVERAGE_THRESHOLD` (default 80) | TECH-TUI-OBSERVABILITY line 1134, AC-OBSERVABILITY-04, NFR-TUI-QUAL-003 |

## Prerequisites

- `check-duplicate-code.sh` requires Node.js (>=18) so that `npx` can fetch `jscpd@4`. The
  first invocation downloads jscpd into the npm cache (~10-30 s); subsequent runs are ~3 s.
  Also requires `python3` for JSON parsing and float threshold comparison.

## Environment Overrides

- `MAX_LINES` (default `500`) — max line count
- `TUI_SRC_ROOT` (default `crates/archon-tui/src`) — source root
- `DEPGRAPH_OVERRIDE` — path to pre-generated DOT file (skips cargo-depgraph invocation, used by self-tests)
- `JSCPD_TARGET_DIR` (default `crates/archon-tui/src`) — target directory for jscpd scan
- `JSCPD_THRESHOLD` (default `5`) — duplication percentage threshold (script fails when `>=` threshold)
- `JSCPD_REPORT_DIR` (default `/tmp/jscpd-report`) — output dir for the jscpd JSON report
- `TUI_GREP_ROOT` (default `crates/ src/`) — search root(s) for `grep-bounded-channel.sh` and `grep-await-send.sh`, space-separated
- `COVERAGE_THRESHOLD` (default `80`) — minimum line-coverage percentage for `check-coverage.sh` (passed to `cargo llvm-cov --fail-under-lines`)

## Self-Tests

Self-tests live in `scripts/ci/tests/`. Run with:

```bash
bash scripts/ci/tests/test-file-size-lint.sh
bash scripts/ci/tests/test-check-duplicate-code.sh
bash scripts/ci/tests/test-grep-gates.sh
bash scripts/ci/tests/test-check-complexity.sh
bash scripts/ci/tests/test-check-coverage.sh
bash scripts/ci/tests/test-workflow-syntax.sh
```

## Workflow Integration

Wired into [`.github/workflows/tui-observability.yml`](../../.github/workflows/tui-observability.yml)
by TASK-TUI-814. The workflow runs on `pull_request` and `push` to `main` when
paths under `crates/archon-tui/**` or `scripts/ci/**` change.

| Script / command | Job id (tui-observability.yml) | Status |
|------------------|--------------------------------|--------|
| `check-tui-file-sizes.sh` | `tui-lint-filesize` | allowed-to-fail |
| `check-complexity.sh` | `tui-lint-complexity` | allowed-to-fail |
| `check-cycles.sh` | `tui-lint-cycles` | allowed-to-fail |
| `check-duplicate-code.sh` | `tui-lint-duplication` | allowed-to-fail |
| `grep-bounded-channel.sh` | `tui-lint-bounded-channel` | **required** |
| `grep-await-send.sh` | `tui-lint-await-send` | **required** |
| `check-coverage.sh` | `tui-coverage` | allowed-to-fail |
| `cargo test -p archon-tui --features load-tests ...` | `tui-load-tests` | allowed-to-fail |
| `cargo test -p archon-tui --lib` | `tui-unit` | **required** |

Allowed-to-fail = `continue-on-error: true` (6 jobs total). Per tech spec
lines 1248-1250, this is the initial state; the modularization-completion task
flips the four lint jobs plus coverage and load tests to required once the
refactor is done.
