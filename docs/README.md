# archon-cli documentation

User-facing documentation for the Rust port of the archon strategic engineering CLI.

## Getting started

- [Quick start](getting-started/quick-start.md) — install, authenticate, run your first agent in 5 minutes
- [Installation](getting-started/installation.md) — full build instructions for macOS, Linux, Windows, WSL2
- [Project setup](getting-started/project-setup.md) — bootstrap a project with `archon-init.sh` (flags, scenarios, troubleshooting)
- [First run](getting-started/first-run.md) — what to expect, where data lives, common gotchas

## Architecture

- [Overview](architecture/overview.md) — workspace crates, request lifecycle, data flow
- [Learning systems](architecture/learning-systems.md) — SONA, ReasoningBank (12 modes), GNN, CausalMemory, Provenance, DESC, Reflexion, AutoCapture
- [Pipelines](architecture/pipelines.md) — god-code (50 agents), god-research (46 agents), agent loop, subagent spawning, multi-agent teams

## Reference

- [Slash commands](reference/slash-commands.md) — 65 primary commands grouped by purpose
- [Tools](reference/tools.md) — 43 built-in tools available to agents
- [Skills](reference/skills.md) — 67 built-in skills (composable command sequences)
- [Permissions](reference/permissions.md) — 7 permission modes, rule lists, sandboxing
- [Configuration](reference/config.md) — `config.toml` schema, precedence, every section
- [CLI flags](reference/cli-flags.md) — every command-line argument
- [Environment variables](reference/env-vars.md) — `ARCHON_*` overrides

## Integrations

- [MCP servers](integrations/mcp-servers.md) — Model Context Protocol transport, registration, discovery
- [Plugins](integrations/plugins.md) — manifest format, lifecycle, packaging
- [Hooks](integrations/hooks.md) — event-driven shell command triggers
- [Identity & spoofing](integrations/identity-spoofing.md) — OAuth, API key, Claude Code mimicry
- [LSP integration](integrations/lsp.md) — language server discovery and operations
- [IDE extensions](integrations/ide-extensions.md) — VS Code, JetBrains protocol

## Cookbook

- [Strategic engagement research](cookbook/strategic-engagement.md) — 22-document intelligence package workflow
- [Memory-driven coding](cookbook/memory-driven-coding.md) — using SONA + ReasoningBank to inform decisions
- [Running god-code pipelines](cookbook/god-code-pipeline.md) — 50-agent coding workflow end-to-end
- [Custom agent workflows](cookbook/custom-agent-workflows.md) — `/create-agent`, `/run-agent`, `/evolve-agent`
- [PRD-driven development](cookbook/prd-driven-development.md) — `/to-prd` → `/prd-to-spec` → `/spec-to-tasks` → `/archon-code`

## Operations

- [Session management](operations/session-management.md) — resume, fork, checkpoint, rewind
- [TUI customization](operations/tui-customization.md) — themes, vim mode, keybindings
- [Cost, effort, fast mode](operations/cost-effort.md) — token tracking, model selection, latency tuning
- [Context compaction](operations/context-compaction.md) — automatic and manual compression
- [Cron & scheduling](operations/cron-scheduling.md) — recurring tasks, one-shot delays
- [Remote control](operations/remote-control.md) — WebSocket server, headless mode, web UI
- [Troubleshooting](operations/troubleshooting.md) — known issues, recovery procedures
- [Data locations](operations/data-locations.md) — where logs, configs, memory, snapshots live
- [Setup wizard](operations/setup-wizard.md) — `/setup-archon-skills` interactive configuration walkthrough

## Development

- [Contributing](development/contributing.md) — workflow, code style, review process
- [Dev flow gates](development/dev-flow-gates.md) — the 6-gate enforcement protocol
- [Adding a tool](development/adding-a-tool.md) — implementing a new built-in tool
- [Adding a skill](development/adding-a-skill.md) — registering a new built-in skill
- [Adding an agent](development/adding-an-agent.md) — flat-file YAML and TOML manifest agents
- [Release process](development/release-process.md) — version bumps, changelog, tagging

## Release notes

- [v0.1.35](release-notes/v0.1.35.md) — Archon skills pack + project installer
- [v0.1.34](release-notes/v0.1.34.md) — Engineering skills pack
- [v0.1.33](release-notes/v0.1.33.md) — Skills foundation (embedded templates, /to-prd, /prd-to-spec)
- [v0.1.28](release-notes/v0.1.28.md) — ReasoningBank 12-mode wire-up + README accuracy sweep
- [v0.1.27](release-notes/v0.1.27.md) — GNN hygiene: early stopping, foreground test hardening
- [v0.1.26](release-notes/v0.1.26.md) — GNN auto-retraining
- [v0.1.25](release-notes/v0.1.25.md) — GNN training infrastructure
- [v0.1.24](release-notes/v0.1.24.md) — GNN forward pass parity with TypeScript reference
- [v0.1.23](release-notes/v0.1.23.md) — Wire all learning systems into production
- [Earlier releases](release-notes/v0.1.6-to-v0.1.13.md) — slash command parity through blocking-lock purge

## Conventions

All claims in these docs are checked against actual code (not aspiration). When code changes, the corresponding doc page is updated in the same PR. Drift detection runs as part of dev flow gates.

If you spot a mismatch between a doc and the code, the doc is wrong. Open an issue or PR; do not assume the code matches the doc.
