# archon-cli documentation

User-facing documentation for the Rust port of the archon strategic engineering CLI.

## Getting started

- [Quick start](getting-started/quick-start.md) — install, authenticate, run your first agent in 5 minutes
- [Installation](getting-started/installation.md) — full build instructions for macOS, Linux, Windows, WSL2
- [Codex authentication](getting-started/codex-auth.md) — ChatGPT/Codex OAuth, TUI provider mode, provider-aware `/model`, tool use, subagents, pipelines
- [Project setup](getting-started/project-setup.md) — bootstrap a project with `archon-init.sh` (flags, scenarios, troubleshooting)
- [First run](getting-started/first-run.md) — what to expect, where data lives, common gotchas

## Architecture

- [Overview](architecture/overview.md) — workspace crates, request lifecycle, data flow
- [Learning systems](architecture/learning-systems.md) — SONA, ReasoningBank (12 modes), GNN, CausalMemory, Provenance, DESC, Reflexion, AutoCapture, and live pipeline/GameTheory wiring
- [Agent task board](architecture/agent-task-board.md) — durable handoffs between agents: why a dedicated Cozo relation rather than a memory type, atomic claims, and why board items are never pruned
- [Cognitive Executive Loop](architecture/cognitive-executive-loop.md) — situation classification, candidate planning, JEPA/world-model scoring, policy gates, reflection, and autonomous tick orchestration
- [Learning systems index](architecture/learning-systems-index.md) — ownership map for memory, completion, world model, reasoning quality, and governed learning signals
- [Reasoning quality](architecture/reasoning-quality.md) — first-class visible claim/evidence events, correction links, critic gates, and briefing warnings
- [Local world model](architecture/world-model.md) — trace corpus, fail-open advisor, dynamic training, retention, backends
- [Pipelines](architecture/pipelines.md) — `/archon-code` (50 agents), `/archon-research` (47 agents / 8 phases), audited bundles, resume verification, agent loop, subagent spawning
- [Dynamic workflows](architecture/dynamic-workflows.md) — provider-neutral generated workflow specs, durable runs, live TUI execution, web workflow view, learning ledgers, resume/restart, and template saving
- [Topology](architecture/topology.md) — the `TaskGraph` IR both workflow specs and subtask batches lower into, the ambient jsonl trace and its batched fold into `.archon/topology.db`, and synchronous guardrail admission (agent cap, single-writer, ungated-irreversible)
- [Evidence Engine](evidence-engine.md) — documents, knowledge, provenance, game theory, completion integrity, governed learning, meaning, constellations

## Evidence Engine

- [Document intelligence](docs.md) — ingest, OCR/VLM policy, embeddings, exact/semantic/hybrid retrieval
- [Video evidence](video.md) — transcript/ASR ingest, frame OCR/VLM, timecode citations, KB consumption
- [Knowledge base](knowledge.md) — claims, entities, relations, source quality, contradictions
- [Trading Lab](trading-lab.md) — governed trading research, strategy specs, Pine prototypes, persistent OHLCV data, deterministic fill/candle/custom-rule backtests, TradingView replay-paper evidence, workflow specs, paper/live gates, risk controls, and audit ledgers
- [Game theory](gametheory.md) — CLI, `/gametheory` slash command, tools, persisted run state
- [Completion integrity](completion-integrity.md) — claims, evidence, incidents, trust scoring
- [Governed learning](governed-learning.md) — learning events, proposals, manifests, approval gates
- [Policy](policy.md) — layered TOML gates for VLM, Tier 11, retrieval, and auto-apply
- [Provenance](provenance.md) — trace, export, verify, document provenance

## Reference

- [Slash commands](reference/slash-commands.md) — 87 primary commands grouped by purpose
- [Tools](reference/tools.md) — 43 built-in tools available to agents
- [Skills](reference/skills.md) — 68 built-in skills (composable command sequences)
- [Permissions](reference/permissions.md) — 7 permission modes, rule lists, sandboxing
- [Configuration](reference/config.md) — `config.toml` schema, precedence, every section
- [Prompt caching and cost](reference/prompt-caching.md) — per-provider wire formats, per-model minimums, breakpoint placement, and how cache reads and writes are priced
- [Cognitive configuration](reference/cognitive-config.md) — `[learning.cognitive]` and `[policy.cognitive]`
- [CLI flags](reference/cli-flags.md) — every command-line argument
- [`archon requirements trace`](reference/requirements-trace.md) — requirement-to-code traceability, the four-level proof ladder, `--leann-db`, and `--falsify`
- [`archon workflow lint`](reference/workflow-lint.md) — the four advisory topology analyses: diamond conformance, edge classification, stop-rule fusion, requirement coverage
- [Environment variables](reference/env-vars.md) — `ARCHON_*` overrides
- [World-model backends](reference/world-model-backends.md) — CPU, CUDA, and MLX Metal support matrix
- [CUDA world-model validation](development/world-model-cuda-validation.md) — local CUDA JEPA validation evidence
- [MLX Metal world-model validation](development/world-model-mlx-metal-validation.md) — Apple Silicon validation checklist
- [World-model embeddings](reference/world-model-embeddings.md) — local and third-party embedding provider matrix
- [Provider capabilities](generated/provider-capabilities.md) — generated Anthropic/Codex surface-support matrix
- [Command surface matrix](generated/command-surface-matrix.md) — generated CLI/slash/TUI parity matrix

## Integrations

- [MCP servers](integrations/mcp-servers.md) — Model Context Protocol transport, registration, discovery
- [Plugins](integrations/plugins.md) — markdown bundles and WASM plugins: layout, lifecycle, packaging
- [Hooks](reference/hooks.md) — all 39 events, the TOML shape and precedence, exit-code semantics, and which events output the runtime actually consumes
- [Identity & spoofing](integrations/identity-spoofing.md) — OAuth, API key, Claude Code mimicry
- [VLM image descriptions](integrations/vlm.md) — Ollama, Gemini, and Anthropic vision providers for image ingest
- [LSP integration](integrations/lsp.md) — language server discovery and operations
- [Java support](integrations/java.md) — cartographer indexing, the Gradle/Maven analysis toolchain, and the java-developer agent
- [IDE extensions](integrations/ide-extensions.md) — VS Code, JetBrains protocol

## Providers

- [Provider runtime](providers/runtime.md) — runtime events, status snapshots, fallback evidence, rate-limit windows
- [Codex provider](providers/codex.md) — direct runtime, app-server JSON-RPC, WebSocket and stdio transports
- [Anthropic Claude Code](providers/anthropic-claude-code.md) — Claude OAuth/API-key routing and spoof compatibility
- [Provider auth profiles](providers/auth-profiles.md) — durable Cozo-backed auth profile import, ordering, health, cooldowns
- [Amazon Bedrock](providers/bedrock.md) — config layering, credentials, model ids and access agreements, the Converse wire format, and verifying the cache
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
- [YouTube video evidence with local Whisper](cookbook/video-evidence-youtube-whisper.md) — governed `yt-dlp` download, caption-first ingest, `ffmpeg`, `whisper-cpp`, optional frame OCR fallbacks, TUI monitoring, and timecoded evidence consumption
- [Multi-agent handoffs](cookbook/multi-agent-handoffs.md) — the task board, claims that expire with their holder, the drain gate that makes "leave no gaps" enforceable, and the third review verdict
- [Agent teams, messaging, and isolation](cookbook/agent-teams-and-isolation.md) — `SendMessage` between agents, status envelopes, declaring writes, the isolation ladder, `/worktrees` review and merge, and `TeamCreate`/`TeamDelete` with a live roster; coding and non-coding recipes
- [Memory-driven coding](cookbook/memory-driven-coding.md) — using SONA + ReasoningBank to inform decisions
- [Coding pipeline (`/archon-code`)](cookbook/god-code-pipeline.md) — 50-agent coding workflow end-to-end inside the TUI
- [Research pipeline (`/archon-research`)](cookbook/archon-research-pipeline.md) — 47-agent PhD research workflow end-to-end inside the TUI
- [Pipeline rewind](cookbook/pipeline-rewind.md) — audited recovery when accepted pipeline outputs are contaminated and must be regenerated
- [Dynamic workflows](cookbook/dynamic-workflows.md) — plan, run, resume, restart-agent, and save generated provider-neutral workflows
- [Trading Lab](cookbook/trading-lab.md) — build trading KBs, create 15-field strategy specs, ingest OHLCV datasets, generate Pine variants, run fill/candle/custom-rule backtests, paper trade, mirror replay evidence through TradingView MCP, generate Trading Lab workflow specs, postmortem, and review live-readiness gates; runnable fixtures live in [`examples/trading-lab/`](../examples/trading-lab/README.md)
- [Game-theory pipeline (`/gametheory`)](cookbook/gametheory-pipeline.md) — Tier 1 classify → route → specialists → report end-to-end inside the TUI
- [Trading and asset analysis with `/gametheory`](cookbook/trading-with-gametheory.md) — applying the game-theory pipeline to pre-trade assessment, post-event decomposition, counterparty analysis, strategy-viability tests, and macro reaction-function modelling
- [World-model and JEPA training](cookbook/world-model-dynamic-training.md) — fresh setup, readiness checks, training, eval, promotion, and idle-aware trainer behavior
- [Proactive session briefing](cookbook/proactive-session-briefing.md) — preview and configure memory, reasoning-quality, proposal, and world-model briefing sections
- [Custom agent workflows](cookbook/custom-agent-workflows.md) — `/create-agent`, `/run-agent`, `/evolve-agent`
- [PRD-driven development](cookbook/prd-driven-development.md) — `/to-prd` → `/prd-to-spec` → `/spec-to-tasks` → `/archon-code`
- [Sandbox real-world use cases](cookbook/sandbox-real-world-use-cases.md) — when to use Docker, OpenShell, SSH, `risky`, `all`, `scratch`, and writable paths

## Operations

- [Session management](operations/session-management.md) — resume, fork, checkpoint, rewind
- [Web workbench](operations/web-workbench.md) — browser interface tabs, data sources, action safety, and setup
- [TUI customization](operations/tui-customization.md) — themes, vim mode, keybindings
- [Cost, effort, fast mode](operations/cost-effort.md) — token tracking, provider-aware model selection, latency tuning
- [Context compaction](operations/context-compaction.md) — automatic and manual compression
- [Cron & scheduling](operations/cron-scheduling.md) — recurring tasks, one-shot delays
- [One-shot context handling](operations/one-shot-context.md) — why bounded calls use classification, not compaction
- [Remote control](operations/remote-control.md) — WebSocket server, SSH, headless mode, and web launch
- [Troubleshooting](operations/troubleshooting.md) — known issues, recovery procedures
- [Data locations](operations/data-locations.md) — where logs, configs, memory, snapshots live
- [SONA workflow tuning](operations/sona-workflow-tuning.md) — which `[workflow.generated]` limits are learned, the five-observation gate, the upward-only timeout ratchet, and how to find out why a run got a given value
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

- [v1.9.1](release-notes/v1.9.1.md) — Patch: prompt caching emitted on every provider rather than only the one endpoint whose URL archon recognised, with `prompt_cache_strategy` to declare a gateway and a guard test that fails the build if a new provider omits its declaration (#178); the stable-head breakpoint placed ahead of the per-turn content on Bedrock and Vertex instead of behind it, where a cache write bills above plain input and is never read back; Vertex system markers no longer flattened away into a string; GPT-5.6 `prompt_cache_breakpoint` with a prefix-derived `prompt_cache_key`; this turn's volatile system blocks moved onto the last user message, the only lever the implicitly-caching providers have (`prompt_cache_reorder`); cache reads and writes priced at 0.1x and 1.25x base input instead of zero and par, per model, with the 10% regional Bedrock and Vertex premium read from the model id and `[context.model_pricing]` to correct any of it without a release; SigV4 canonical paths encoded twice, without which every dated Bedrock model id failed as a bad secret key; `prompt_cache_conversation = false` no longer discarding the tools and system checkpoints along with the message one; OpenAI cached tokens no longer counted twice into context pressure and cost; an EC2 instance profile no longer reported as missing credentials; and `RUST_LOG` and `ARCHON_DEBUG_LOG_DIR` reaching the session log at all. Verified live on five Bedrock models, Anthropic subscription OAuth, and the OpenAI Codex subscription
- [v1.9.0](release-notes/v1.9.0.md) — Minor: knowledge bases unified into one namespace, so a name created on any surface is listed and usable from all of them, and a name nobody wrote down is recoverable at all (#170); corrections reinforcing a rule only once something has been shown to have caused them, with causal attribution running in shadow and failing closed (#77); scheduled Memory Garden consolidation under a single-run lock and a work budget, with irreversible pruning routed to governed proposals instead of deleting (#79); replay weighted by the latent surprise that was being recorded and never read, bounded three ways and held out by session (#85); the subagent runtime's per-round copies cut from five to two, transcripts opened once instead of once per message, and a ten-way fan-out querying the memory store three times rather than thirty, with request bytes verified byte-identical (#171); multi-line TUI input via Shift+Enter with the enhancement flags popped on every exit path, and the leftover cells that put stray glyphs in the input area (#174); a policy refusal in the web workbench answering 403 rather than 200 (#170); RocksDB's periodic stats dump disabled, which was causing a null dereference at exit on Linux and a deadlock on Windows; and `reports/` untracked
- [v1.8.0](release-notes/v1.8.0.md) — Minor: cognitive release gates judged per cohort, blocking `propose_improvements` and exiting non-zero from `archon cognitive gate` (#83); a self-model prediction written before the action it predicts, and a briefing that names what it does not know (#80); reflections carried forward under three bounds and measured on verified reuse, replacing a citation rate that was 1.0 by construction (#81); an R0 entry gate that can go red, wired into CI and `ci-gate.sh` (#86); sandboxed PDF rendering in the corpus viewer under a `script-src 'self' 'wasm-unsafe-eval'` CSP with scripting disabled (#160); web-chat attachments reporting the docs `document_id` they were ingested as (#164); workflow stage branches reaching the task board over both V2 dispatch paths (#161); declared artifacts required to be regular non-empty files, and prose refused as a required-artifact path (#168); tools no longer hoisted into every task by the project capability manifest (#163); `blocking_gap_detected` in `events.jsonl` so a lost remediation wave stops presenting as healthy (#162); seven web surfaces refetching on the live events that change them (#165); two production `/tmp` fallbacks no longer writing to the drive root (#156); and the hook-executor half of #154, where spawn time was charged to the hook's work budget and a passed deadline could be outvoted by an exit code
- [v1.7.0](release-notes/v1.7.0.md) — Minor: the executive loop, reflection writer, self-model writer and an abstaining correction classifier all wired to live call sites after being built and never called; `docs compile`/`answer`/`export` giving REQ-KB-002 and REQ-KB-003 commands to reach them, with streamed synthesis; VSCode tool execution behind a permission round-trip that refuses; a content-hash dedup race closed in the shipping ingest path (#155); `CognitiveTick` no longer reporting success for work it never did (#153); test helpers no longer writing SQLite databases to the drive root (#156); five hand-rolled fence strippers consolidated, two of which were silently losing data; and every file under the 500-line ceiling for the first time in months
- [v1.6.1](release-notes/v1.6.1.md) — Patch: document ingest was crashing outright on any OpenAI-compatible embedding provider, the TUI panicked on terminals under ~22 rows, plus a browser terminal pane running the real TUI, knowledge-store upload/delete/index controls, three panels that fetched data and rendered none of it, a RapidOCR virtualenv the setup scripts never created, Git's shell being found on Windows from any `git.exe` layout (#118), and a screenshot and description for every workbench tab
- [v1.6.0](release-notes/v1.6.0.md) — Agent task board with atomic claims, subagent identity in ToolContext, self-checking edits via PostToolUse, an in-process web dashboard with a board view, a fix for semantic dedup silently no-opping on a second instance, and the repository code index no longer building itself at session start (`[code_index] index_on_startup`, now off by default)
- [v1.5.2](release-notes/v1.5.2.md) — Patch: contain Marker HTTP PDF access behind a frozen opaque-ID catalogue, harden custom-regex validation, fix bounded memory recall/writes/consolidation and resumed-session continuity, expose `/memory store` and automatic consolidation summaries, and update security-sensitive dependencies
- [v1.5.1](release-notes/v1.5.1.md) — Patch: cancelling a turn no longer wedges the session (admission claims leaked on every cancel), memory recall is bounded and moved off the async executor (38 minutes of one core per turn on a 1.7 GB store), shifted symbols and non-ASCII characters type on non-US keyboards, and the LEANN indexer's exclusions actually exclude
- [v1.5.0](release-notes/v1.5.0.md) — Topology layer (`TaskGraph` IR, ambient trace, guardrail admission), `archon requirements trace` with a four-level proof ladder and `--falsify`, advisory `archon workflow lint`, `/workflow-prd` and `/workflow-prd-spec` skills, SONA tuning of the generated-workflow limits plus a learned fan-out width, and two behaviour changes: hooks resolve a POSIX shell on Windows, and `ARCHON_DATA_DIR` means one directory
- [v1.4.0](release-notes/v1.4.0.md) — Self-learning JEPA world model (trainable encoders, dense embedding input, live advisory loop, automatic candidate lifecycle), `archon draft` FCDP pipeline with provenance import, agent catalog snapshot fixes, and a repository-wide 500-line file ceiling
- [v1.3.11](release-notes/v1.3.11.md) — Governed Trading Lab substrate with strategy specs, Pine prototypes, persistent OHLCV data, fill/candle/custom-rule backtests, paper/live gates, TradingView replay-paper evidence, workflow specs, risk/audit controls, learning hooks, and user/cookbook documentation
- [v1.3.10](release-notes/v1.3.10.md) — Provider-neutral dynamic workflows, durable workflow bundles, live TUI Agent Activity, web Workflows page, learning ledgers, and `/workflow` CLI/TUI control
- [v1.3.9](release-notes/v1.3.9.md) — RocksDB document vector store, resumable legacy-vector migration, Rust-HNSW compaction, durable index queue/daemon controls, and vector diagnostics
- [v1.3.8](release-notes/v1.3.8.md) — Cognitive Executive Loop, autonomous cognitive ticks, opt-in Rust daemon, executive-state CLI/TUI/web surfaces, and safety-gated self-model/world-model coordination
- [v1.3.7](release-notes/v1.3.7.md) — Autonomous governed-learning tick, policy-gated self-application, provider-resolved pipeline subagent activity, and updated learning docs
- [v1.3.6](release-notes/v1.3.6.md) — Video evidence capture fallbacks: caption-first ingest, frame-friendly `yt-dlp`, OpenCV frame fallback, and RapidOCR image/frame OCR
- [v1.3.5](release-notes/v1.3.5.md) — Governed YouTube/video ingest hardening, local `whisper-cpp` ASR chunks, PNG frame extraction, frame reprocess, and cross-OS setup docs
- [v1.3.4](release-notes/v1.3.4.md) — DeepSeek provider parity, Anthropic-compatible DeepSeek sessions, generic non-Claude model switching, and provider-aware cost display
- [v1.3.3](release-notes/v1.3.3.md) — Governed video evidence ingest, `/video` slash parity, transcript/frame evidence storage, and policy-gated video summaries
- [v1.3.2](release-notes/v1.3.2.md) — Finalize JEPA eval quick/full mode, promotion provenance, eval-run status commands, and source-size cleanup
- [v1.3.1](release-notes/v1.3.1.md) — Add JEPA world-model training/runtime, normal-session and pipeline guardrails, CUDA/MLX accelerator support, and fresh-setup cookbook docs
- [v1.3.0](release-notes/v1.3.0.md) — Remove model-facing subagent `max_turns`, harden compaction persistence/request-pressure recovery, add lazy agent catalog, and improve context/status signals
- [v1.2.9](release-notes/v1.2.9.md) — Autocompaction trigger correctness + proactive soft-fail with fall-through; reactive remains fatal
- [v1.2.8](release-notes/v1.2.8.md) — Complete auto-compaction PRD closure: structured summaries, prompt budgets, context UI, and prompt-cache policy
- [v1.2.7](release-notes/v1.2.7.md) — Resume-state split tool-result repair and configurable context-window catalog
- [v1.2.6](release-notes/v1.2.6.md) — Symmetric tool-pair repair for Anthropic-shape and Codex Responses providers
- [v1.2.5](release-notes/v1.2.5.md) — Provider tier alias resolution + Anthropic message sanitizer + pair-safe compaction
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
