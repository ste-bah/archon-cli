# Maintainability Refactor Map

Generated: 2026-05-12

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
| 4 | Core agents, hooks, config, and patterns | `agents/loader.rs`, `agents/memory.rs`, `hooks/registry.rs`, `config.rs` | Agent discovery/load, memory prompts, hook summaries, config reload |
| 5 | Command surface and CLI registry | `src/command/registry.rs`, `src/cli_args.rs` | 81 primary commands, aliases, help output, slash mirror |
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

Group 3 is functionally complete but now carries one temporary re-allowlisted regression: `crates/archon-core/src/agent.rs` has drifted back up to 525 lines and has been re-added to `scripts/check-file-sizes.allowlist` so the guard stays green while a follow-up trim is scheduled. `crates/archon-core/src/subagent.rs` and `crates/archon-core/src/subagent_executor.rs` remain below the threshold.

Group 5 is complete: `src/command/registry.rs` is now a 19-line compatibility shell backed by focused `src/command/registry/*` modules, and `src/cli_args.rs` is now a 24-line compatibility shell backed by focused `src/cli_args/*` modules. Both files have been removed from `scripts/check-file-sizes.allowlist`.

Group 4 was missing from the earlier map and is now restored ahead of Group 6. Group 4 is now in progress: `crates/archon-core/src/agents/loader.rs` has been split from 2020 lines to a 29-line compatibility shell, and `crates/archon-core/src/agents/memory.rs` has been split from 1207 lines to a 22-line compatibility shell. Both are backed by focused submodules and removed from `scripts/check-file-sizes.allowlist`.

Group 6 is now materially underway. `src/session.rs` has been reduced to a 439-line orchestration shell backed by focused session-phase modules:

- `src/session/interactive_bootstrap.rs` — 415 lines
- `src/session/interactive_setup.rs` — 217 lines
- `src/session/interactive_agent.rs` — 461 lines
- `src/session/interactive_finish.rs` — 237 lines
- `src/session/interactive_ui.rs` — 251 lines
- supporting helpers: `build_agent.rs` (462), `build_prompt.rs` (220), `event_forwarder.rs` (235), `config_watcher.rs` (59), `slash_context_builder.rs` (111), `btw.rs` (68), `splash.rs` (48)

The remaining Group 6 production targets are `src/session_loop/mod.rs` (817) and `src/main.rs` (690).

The next high-value maintainability targets are:

1. Re-close the temporary Group 3 carryover in `crates/archon-core/src/agent.rs` (525, currently allowlisted)
2. Continue Group 4 core agents, hooks, config, and patterns work, especially `crates/archon-core/src/config.rs` and `crates/archon-core/src/hooks/registry.rs`
3. Finish Group 6 by splitting `src/session_loop/mod.rs` and `src/main.rs`

## Verification Matrix

| Touched area | Required checks |
|---|---|
| File-size docs or allowlist only | `bash scripts/check-file-sizes.sh`, raw allowlist count, `git diff --check` |
| Gametheory facade/registry/routing | `CARGO_BUILD_JOBS=1 cargo test -p archon-pipeline -j1 gametheory:: -- --test-threads=1` |
| Core agent/subagent/config/hooks | `CARGO_BUILD_JOBS=1 cargo test -p archon-core -j1 -- --test-threads=1` |
| Core agents/hooks/config Group 4 slices | `CARGO_BUILD_JOBS=1 cargo test -p archon-core -j1 agents:: -- --test-threads=1`, `CARGO_BUILD_JOBS=1 cargo test -p archon-core -j1 hooks:: -- --test-threads=1`, or focused adjacent package checks |
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
