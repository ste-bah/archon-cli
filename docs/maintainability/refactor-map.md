# Maintainability Refactor Map

Generated: 2026-05-07

Source PRD: `PRD-ARCHON-FINALISATION-003`

This map assigns each oversized subsystem to a reviewable refactor group. The order favours behaviour-risk first, then allowlist reduction.

## Rules

- Preserve public imports with `pub use` compatibility shells where callers depend on existing paths.
- Keep new Rust modules at or below 500 lines.
- Remove allowlist entries in the same change that brings a file below 500 lines.
- Run one package-filtered Cargo command at a time with WSL-safe limits: `CARGO_BUILD_JOBS=1`, `-j1`, and `--test-threads=1` where applicable.
- Do not change CLI, TUI, provider, persistence, or pipeline behaviour as a side effect of a split.

## Group Order

| Group | Scope | First targets | Source of truth |
|---|---|---|---|
| 0 | Inventory, guard hygiene, refactor map | `scripts/check-file-sizes.allowlist`, `docs/maintainability/*` | File-size guard output and raw allowlist count |
| 1 | Gametheory facade | `crates/archon-pipeline/src/gametheory/facade.rs` | `gt_runs`, `gt_specialist_outputs`, reports, classify/replay output |
| 2 | Gametheory registry and routing | `registry.rs`, `routing.rs`, `schema.rs` | 84-agent coverage, routing YAML, DAG cycle/orphan checks |
| 9 | Memory, learning and GNN | `learning/gnn/trainer.rs`, `auto_trainer.rs`, `integration.rs` | GNN weights, Adam state, training runs, correction events |
| 3 | Core agent and subagent runtime | `agent.rs`, `subagent.rs`, `subagent_executor.rs` | Agent launch, cancellation, event streaming |
| 5 | Command surface and CLI registry | `src/command/registry.rs`, `src/cli_args.rs` | 78 primary commands, aliases, help output, slash mirror |
| 6 | Session startup and loop | `src/session.rs`, `src/session_loop/mod.rs`, `src/main.rs` | TUI/headless startup, shutdown, provider and auto-trainer wiring |
| 7 | Document retrieval and store | `archon-docs/src/retrieval.rs`, `store.rs` | Cozo doc rows, search order, VLM/image ingest |
| 8 | Pipeline runtime, coding and research | coding, research, executor, runner, KB modules | `/archon-code`, `/archon-research`, quality gates, pipeline persistence |
| 10 | Provider, LLM and tool surface | `archon-llm`, `archon-tools`, `providers.rs` | Redacted auth reports, captured headers, provider capability matrix |
| 11 | Secondary commands and test debt | remaining command modules and oversized tests | Command-specific tests, snapshots, helper reuse |

## Priority Notes

Group 9 is pulled forward from the original plan because v0.1.51 and v0.1.52 added correction events, hydrated meaning triplets, GNN auto-training, and `/learning gnn status`. That work expanded already-oversized learning and agent/session modules.

Group 5 and Group 10 both touch `src/command/providers.rs`. Treat that file as provider-surface code owned by Group 10, but coordinate any slash/registry wiring with Group 5.

Group 11 should not become a dumping ground. Split tests alongside adjacent production groups where possible; use Group 11 only for command/test debt with no higher-risk subsystem owner.

## Current Status

Group 3 is complete in this worktree: `crates/archon-core/src/agent.rs` is 489 lines, `crates/archon-core/src/subagent.rs` is 370 lines, and `crates/archon-core/src/subagent_executor.rs` is 286 lines. All three have been removed from `scripts/check-file-sizes.allowlist`. Continue with Group 5 command-surface targets.

## Verification Matrix

| Touched area | Required checks |
|---|---|
| File-size docs or allowlist only | `bash scripts/check-file-sizes.sh`, raw allowlist count, `git diff --check` |
| Gametheory facade/registry/routing | `CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline -j1 gametheory:: -- --test-threads=1` |
| Core agent/subagent/config/hooks | `CARGO_BUILD_JOBS=1 cargo test -p archon-core -j1 -- --test-threads=1` |
| Command registry or CLI args | `CARGO_BUILD_JOBS=1 cargo test -p archon-cli-workspace -j1 command -- --test-threads=1` plus help/surface checks |
| Session/TUI loop | `CARGO_BUILD_JOBS=1 cargo test -p archon-tui -j1 -- --test-threads=1` and shutdown smoke tests |
| Docs retrieval/store/VLM | `CARGO_BUILD_JOBS=1 cargo test -p archon-docs -j1 -- --test-threads=1` |
| Learning/GNN/meaning | `CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline -j1 learning::gnn -- --test-threads=1`, `cargo test -p archon-meaning -j1 -- --test-threads=1` |
| Provider/LLM/tool surface | `CARGO_BUILD_JOBS=1 cargo test -p archon-llm -j1 -- --test-threads=1`, provider doctor smoke |

## Commit Evidence Template

Every split commit should include:

```text
Group: <group number and name>

Split:
- <old file> <old lines> -> <new module set and line counts>

Allowlist:
- active before: <N>
- active after: <M>
- raw before: <N>
- raw after: <M>
- removed: <paths>

Behaviour:
- public API preserved: yes/no
- intentional deviations: <none or list>

Verification:
- <exact command> -> <result>
- bash scripts/check-file-sizes.sh -> 0 over 500, <N> allowlisted
- git diff --check -> clean

Source-of-truth evidence:
- <row counts / command output / registry count / fixture result>
```
