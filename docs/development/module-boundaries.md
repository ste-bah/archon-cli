# Module Boundaries

Generated: 2026-05-07

This guide keeps future feature work from flowing back into the files retired by `PRD-ARCHON-FINALISATION-003`.

## Boundary Rules

- Put new behaviour in the subsystem module that owns the runtime concern.
- Keep public compatibility shells thin: imports, re-exports, and orchestration only.
- Do not add unrelated helpers to `utils.rs` or `helpers.rs`; name modules by responsibility.
- New or moved Rust files should stay at or below 500 lines.
- If a public path moves, preserve the old path with a re-export unless a migration is documented.

## Runtime Boundaries

| Area | Future code belongs in | Avoid adding to |
|---|---|---|
| Gametheory tier execution | `crates/archon-pipeline/src/gametheory/facade/*` by tier or stage | `gametheory/facade.rs` orchestration shell |
| Gametheory registry metadata | `gametheory/registry/*` split by agent data, validation, tiers, metadata | `gametheory/registry.rs` |
| Gametheory routing | `gametheory/routing/*` split by spec parsing, conditions, DAG planning | `gametheory/routing.rs` |
| Agent launch/runtime | `crates/archon-core/src/agent/*` split by config, context, runtime, tool loop, streaming | `crates/archon-core/src/agent.rs` |
| Subagent lifecycle | `crates/archon-core/src/subagent/*` split by spawn, lifecycle, scheduler, results | `crates/archon-core/src/subagent.rs` |
| Agent discovery/loading | `crates/archon-core/src/agents/*` split by discovery, parse, validation, runtime registration | `agents/loader.rs` |
| Config | `crates/archon-core/src/config/*` split by defaults, env overlays, TOML, validation | `config.rs` |
| Hook registry | `crates/archon-core/src/hooks/*` split by registration, validation, execution | `hooks/registry.rs` |
| CLI arguments | `src/cli_args/*` split by command family | `src/cli_args.rs` |
| Slash command registry | `src/command/registry/*` split by command family and mirror metadata | `src/command/registry.rs` |
| Provider doctor/status | `src/command/providers/*` split by doctor rendering, live checks, auth reports, capabilities | `src/command/providers.rs` |
| Session startup | `src/session/*` split by startup, auth, providers, VLM, learning, TUI, shutdown | `src/session.rs` |
| Session event loop | `src/session_loop/*` split by input, MCP events, slash commands, TUI events, rendering | `src/session_loop/mod.rs` |
| Docs retrieval | `crates/archon-docs/src/retrieval/*` split by exact, semantic, hybrid, rerank, reindex, debug | `retrieval.rs` |
| Docs store | `crates/archon-docs/src/store/*` split by documents, pages, chunks, embeddings, provenance, images | `store.rs` |
| Pipeline coding/research | `crates/archon-pipeline/src/coding/*` and `research/*` split by agents, facade, gates, quality, style | single-file facades that own prompt, execution, and persistence together |
| Learning/GNN | `crates/archon-pipeline/src/learning/gnn/*` split by model, trainer, triplet loss, auto-trainer, weights, telemetry | `learning/gnn/trainer.rs` or `learning/gnn/mod.rs` as catch-alls |
| Memory graph | `crates/archon-memory/src/*` split by schema, graph operations, access policy, garden workflows | `graph.rs` |
| Provider auth/transport | `crates/archon-llm/src/providers/*` split auth/header construction from request transport and registry metadata | provider files that mix capability metadata, auth, and HTTP |
| Tool surface | `crates/archon-tools/src/*` split tool input validation, execution, rendering, and tests | monolithic tool files |

## Preserved Public Surfaces

Keep these surfaces stable during refactors:

- CLI command names, aliases, and help output.
- Slash registry command count and mirror metadata.
- TUI command-center behaviour and event streaming.
- Provider doctor/model-status output unless the change is explicitly documented.
- Cozo relation names and persisted row shapes.
- Gametheory report artifact paths and persisted run rows.
- GNN weight, Adam-state, training-run, correction-event, and hydrated-triplet flows.

## Allowlist Discipline

When a file drops to 500 lines or below:

1. Remove its path from `scripts/check-file-sizes.allowlist`.
2. Run `bash scripts/check-file-sizes.sh`.
3. Update `docs/maintainability/file-size-inventory.md` if the change is part of a maintainability slice.
4. Report active and raw allowlist counts in the commit message.

Do not add a new allowlist entry without a dated justification and explicit owner.

