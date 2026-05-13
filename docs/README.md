# archon-cli documentation

User-facing documentation for the Rust port of the archon strategic engineering CLI.

## Getting started

- [Quick start](getting-started/quick-start.md) — install, authenticate, run your first agent in 5 minutes
- [Installation](getting-started/installation.md) — full build instructions for macOS, Linux, Windows, WSL2
- [Codex authentication](getting-started/codex-auth.md) — ChatGPT/Codex OAuth, TUI provider mode, tool use, subagents, pipelines
- [Project setup](getting-started/project-setup.md) — bootstrap a project with `archon-init.sh` (flags, scenarios, troubleshooting)
- [First run](getting-started/first-run.md) — what to expect, where data lives, common gotchas

## Architecture

- [Overview](architecture/overview.md) — workspace crates, request lifecycle, data flow
- [Learning systems](architecture/learning-systems.md) — SONA, ReasoningBank (12 modes), GNN, CausalMemory, Provenance, DESC, Reflexion, AutoCapture
- [Learning systems index](architecture/learning-systems-index.md) — ownership map for memory, completion, world model, reasoning quality, and governed learning signals
- [Reasoning quality](architecture/reasoning-quality.md) — first-class visible claim/evidence events, correction links, critic gates, and briefing warnings
- [Local world model](architecture/world-model.md) — trace corpus, fail-open advisor, dynamic training, retention, backends
- [Pipelines](architecture/pipelines.md) — `/archon-code` (50 agents), `/archon-research` (46 agents), audited bundles, resume verification, agent loop, subagent spawning
- [Evidence Engine](evidence-engine.md) — documents, knowledge, provenance, game theory, completion integrity, governed learning, meaning, constellations

## Evidence Engine

- [Document intelligence](docs.md) — ingest, OCR/VLM policy, embeddings, exact/semantic/hybrid retrieval
- [Knowledge base](knowledge.md) — claims, entities, relations, source quality, contradictions
- [Game theory](gametheory.md) — CLI, `/gametheory` slash command, tools, persisted run state
- [Completion integrity](completion-integrity.md) — claims, evidence, incidents, trust scoring
- [Governed learning](governed-learning.md) — learning events, proposals, manifests, approval gates
- [Policy](policy.md) — layered TOML gates for VLM, Tier 11, retrieval, and auto-apply
- [Provenance](provenance.md) — trace, export, verify, document provenance

## Reference

- [Slash commands](reference/slash-commands.md) — 80 primary commands grouped by purpose
- [Tools](reference/tools.md) — 43 built-in tools available to agents
- [Skills](reference/skills.md) — 68 built-in skills (composable command sequences)
- [Permissions](reference/permissions.md) — 7 permission modes, rule lists, sandboxing
- [Configuration](reference/config.md) — `config.toml` schema, precedence, every section
- [CLI flags](reference/cli-flags.md) — every command-line argument
- [Environment variables](reference/env-vars.md) — `ARCHON_*` overrides
- [World-model backends](reference/world-model-backends.md) — CPU, CUDA, and MLX Metal support matrix
- [World-model embeddings](reference/world-model-embeddings.md) — local and third-party embedding provider matrix
- [Provider capabilities](generated/provider-capabilities.md) — generated Anthropic/Codex surface-support matrix
- [Command surface matrix](generated/command-surface-matrix.md) — generated CLI/slash/TUI parity matrix

## Integrations

- [MCP servers](integrations/mcp-servers.md) — Model Context Protocol transport, registration, discovery
- [Plugins](integrations/plugins.md) — manifest format, lifecycle, packaging
- [Hooks](integrations/hooks.md) — event-driven shell command triggers
- [Identity & spoofing](integrations/identity-spoofing.md) — OAuth, API key, Claude Code mimicry
- [VLM image descriptions](integrations/vlm.md) — Ollama, Gemini, and Anthropic vision providers for image ingest
- [LSP integration](integrations/lsp.md) — language server discovery and operations
- [IDE extensions](integrations/ide-extensions.md) — VS Code, JetBrains protocol

## Providers

- [Provider runtime](providers/runtime.md) — runtime events, status snapshots, fallback evidence, rate-limit windows
- [Codex provider](providers/codex.md) — direct runtime, app-server JSON-RPC, WebSocket and stdio transports
- [Anthropic Claude Code](providers/anthropic-claude-code.md) — Claude OAuth/API-key routing and spoof compatibility
- [Provider auth profiles](providers/auth-profiles.md) — durable Cozo-backed auth profile import, ordering, health, cooldowns
- [Cloud and local providers](providers/cloud-and-local.md) — Anthropic, Bedrock, Vertex, Gemini, local, and compatible routes
- [OpenAI-compatible providers](providers/openai-compatible.md) — compatible API-key endpoints and provider-neutral observation

## Agents and learning

- [Governed agent evolution](agents/evolution.md) — proposal, shadow, apply, reject, rollback, history, status
- [Memory system promotion](agents/memory-system-promotion.md) — promoting candidates into Archon's memory system without markdown files
- [Agent permission governance](agents/permission-governance.md) — profile permission diffs and tool-access review
- [Governed agent evolution storage](learning/governed-agent-evolution.md) — Cozo-backed ledgers, proposals, profile versions, shadow evaluations

## Security and sandboxing

- [Sandboxing](security/sandboxing.md) — backend model, safety posture, and routing decisions
- [Sandbox cookbook](cookbook/sandbox-real-world-use-cases.md) — plain-English real-world Docker, OpenShell, SSH, and mode examples
- [Tool preflight](security/tool-preflight.md) — pre-execution permission and sandbox checks
- [Docker sandbox](security/docker-sandbox.md) — Docker backend policy, mounts, and diagnostics
- [SSH sandbox](security/ssh-sandbox.md) — SSH backend policy, routing, and diagnostics
- [OpenShell sandbox](security/openshell-sandbox.md) — OpenShell backend policy and spoof-safety notes

## Cookbook

- [Strategic engagement research](cookbook/strategic-engagement.md) — 22-document intelligence package workflow
- [Real-world Evidence Engine examples](cookbook/real-world-evidence-engine.md) — research, education, business, trading, coding, and strategic analysis workflows
- [Memory-driven coding](cookbook/memory-driven-coding.md) — using SONA + ReasoningBank to inform decisions
- [Coding pipeline (`/archon-code`)](cookbook/god-code-pipeline.md) — 50-agent coding workflow end-to-end inside the TUI
- [Research pipeline (`/archon-research`)](cookbook/archon-research-pipeline.md) — 46-agent PhD research workflow end-to-end inside the TUI
- [Game-theory pipeline (`/gametheory`)](cookbook/gametheory-pipeline.md) — Tier 1 classify → route → specialists → report end-to-end inside the TUI
- [Trading and asset analysis with `/gametheory`](cookbook/trading-with-gametheory.md) — applying the game-theory pipeline to pre-trade assessment, post-event decomposition, counterparty analysis, strategy-viability tests, and macro reaction-function modelling
- [World-model dynamic training](cookbook/world-model-dynamic-training.md) — backfill, cold-start gates, idle-aware trainer, backend selection
- [Proactive session briefing](cookbook/proactive-session-briefing.md) — preview and configure memory, reasoning-quality, proposal, and world-model briefing sections
- [Custom agent workflows](cookbook/custom-agent-workflows.md) — `/create-agent`, `/run-agent`, `/evolve-agent`
- [PRD-driven development](cookbook/prd-driven-development.md) — `/to-prd` → `/prd-to-spec` → `/spec-to-tasks` → `/archon-code`
- [Sandbox real-world use cases](cookbook/sandbox-real-world-use-cases.md) — when to use Docker, OpenShell, SSH, `risky`, `all`, `scratch`, and writable paths

## Operations

- [Session management](operations/session-management.md) — resume, fork, checkpoint, rewind
- [Web workbench](operations/web-workbench.md) — browser interface tabs, data sources, action safety, and setup
- [TUI customization](operations/tui-customization.md) — themes, vim mode, keybindings
- [Cost, effort, fast mode](operations/cost-effort.md) — token tracking, model selection, latency tuning
- [Context compaction](operations/context-compaction.md) — automatic and manual compression
- [Cron & scheduling](operations/cron-scheduling.md) — recurring tasks, one-shot delays
- [Remote control](operations/remote-control.md) — WebSocket server, SSH, headless mode, and web launch
- [Troubleshooting](operations/troubleshooting.md) — known issues, recovery procedures
- [Data locations](operations/data-locations.md) — where logs, configs, memory, snapshots live
- [Setup wizard](operations/setup-wizard.md) — `/setup-archon-skills` interactive configuration walkthrough
- [TUI drain-stall warning](operations/tui-drain-stall.md) — what the render-loop stall warning means

## Development

- [Contributing](development/contributing.md) — workflow, code style, review process
- [Dev flow gates](development/dev-flow-gates.md) — the 6-gate enforcement protocol
- [Adding a tool](development/adding-a-tool.md) — implementing a new built-in tool
- [Adding a skill](development/adding-a-skill.md) — registering a new built-in skill
- [Adding an agent](development/adding-an-agent.md) — flat-file YAML and TOML manifest agents
- [Release process](development/release-process.md) — version bumps, changelog, tagging
- [World-model handover](development/world-model-handover.md) — implementation notes, checks, continuation pointers
- [Reasoning-quality implementation tasks](development/reasoning-quality-implementation-tasks.md) — milestone status for PRD006C

## Release notes

- [v1.2.4](release-notes/v1.2.4.md) — Metrics provider event tail + ledger activity panel
- [v1.2.3](release-notes/v1.2.3.md) — Browser web workbench for local Archon inspection and operations
- [v1.2.2](release-notes/v1.2.2.md) — Provider-aware auto-compaction
- [v1.2.1](release-notes/v1.2.1.md) — TUI cancellation and TaskCreate lifecycle fixes
- [v1.2.0](release-notes/v1.2.0.md) — Local trace world model plus reasoning-quality events and proactive briefing
- [v1.1.0-beta.3](release-notes/v1.1.0-beta.3.md) — Provider runtime governance and governed agent evolution (supersedes unpublished v1.1.0-beta.1 and v1.1.0-beta.2)
- [v1.0.1](release-notes/v1.0.1.md) — Provider-neutral hybrid retrospective analysis
- [v1.0.0](release-notes/v1.0.0.md) — Audited pipeline runtime and self-calibration
- [v0.1.52](release-notes/v0.1.52.md) — Learning systems completion
- [v0.1.51](release-notes/v0.1.51.md) — Corrections feed behavioural-rule proposals
- [v0.1.50](release-notes/v0.1.50.md) — VS Code extension install fix
- [v0.1.49](release-notes/v0.1.49.md) — TUI drain-stall false positive fix
- [v0.1.48](release-notes/v0.1.48.md) — OpenAI-compatible VLM and Gemini retry hardening
- [v0.1.47](release-notes/v0.1.47.md) — Unified PDF text, OCR, and VLM image ingest
- [v0.1.46](release-notes/v0.1.46.md) — Multi-provider VLM image descriptions
- [v0.1.40](release-notes/v0.1.40.md) — Codex OAuth docs, Claude OAuth spoof continuity, TUI agent activity rail
- [v0.1.39](release-notes/v0.1.39.md) — Evidence Engine PRD compliance pass
- [v0.1.36](release-notes/v0.1.36.md) — trajectory embeddings + persistence
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
