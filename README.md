# archon-cli

<div align="center">
  <img src="archon-avatar.png" alt="Archon Avatar" width="600" />
</div>

A strategic engineering CLI built in Rust. Self-learning agent platform with
persistent memory, multi-agent pipelines, Evidence Engine provenance, document
intelligence, governed learning, local world-model advisory learning,
reasoning-quality events, Trading Lab data/backtest controls, and
identity-aware Anthropic/Codex provider integration.

> **Documentation has moved.** This README is now a landing page. The full structured docs live in [`docs/`](docs/README.md) — start there.

---

## Quick start

```bash
# Build (rustup installs the pinned toolchain from rust-toolchain.toml)
git clone https://github.com/ste-bah/archon-cli
cd archon-cli
sudo scripts/install-system-deps.sh --check || sudo scripts/install-system-deps.sh
# macOS/Homebrew: run scripts/install-system-deps.sh without sudo.
# Optional sandbox deps: sudo scripts/install-system-deps.sh --with-sandbox
cargo build --release --bin archon
ARCHON_BIN="$(pwd)/target/release/archon"

# Authenticate with Claude/Anthropic OAuth or API-key billing
"$ARCHON_BIN" auth login --provider anthropic
# or: export ANTHROPIC_API_KEY="sk-ant-api..."

# Optional: authenticate with a ChatGPT/Codex subscription
"$ARCHON_BIN" auth login --provider openai-codex
"$ARCHON_BIN" auth status

# Optional: store a Google Gemini API key for cloud VLM image descriptions
"$ARCHON_BIN" auth login --provider google

# Initialise a blank project directory
mkdir -p ~/projects/my-archon-project
sh scripts/archon-init.sh \
  --target ~/projects/my-archon-project \
  --archon-cli-repo "$(pwd)"

# Run interactive TUI from the project root
cd ~/projects/my-archon-project
"$ARCHON_BIN"

# Non-interactive print mode
"$ARCHON_BIN" -p "summarize this project layout" --output-format json

# Browser workbench
"$ARCHON_BIN" web --port 8421 --bind-address 127.0.0.1
```

WSL2 builders: add `-j1` to avoid OOM during compilation.

Full installation guide: [`docs/getting-started/installation.md`](docs/getting-started/installation.md).

## What archon-cli is

| | claude-code (TS/Bun) | archon-cli (Rust) |
|---|---|---|
| Runtime | TypeScript / Bun | Rust (pinned via rust-toolchain.toml) |
| Memory | markdown files | CozoDB graph + embeddings |
| Documents | ad hoc file reads | OCR, image ingest, chunks, embeddings, hybrid retrieval, citations |
| Pipelines | Single-agent loop | 50-agent coding + 46-agent research + 84-specialist game theory |
| Reasoning | Direct LLM call | 12 reasoning modes (deductive, inductive, abductive, analogical, adversarial, counterfactual, temporal, constraint, decomposition, first-principles, causal, contextual) |
| Learning | None | 8 subsystems plus local world-model advisory learning and first-class reasoning-quality events |
| Verification | model self-report | completion evidence, false-completion incidents, trust scores, provenance traces |
| Identity | Native | Claude Code spoof, Anthropic OAuth/API keys, or Codex OAuth |

archon-cli is **not affiliated with Anthropic or OpenAI**. It can use an Anthropic API key, Anthropic/Claude OAuth with Claude Code identity spoofing, or OpenAI Codex OAuth where that provider is selected.

## Authentication

Archon has two subscription-auth paths plus normal API keys. Gemini VLM uses a Google API key, stored in the same credentials file when you choose the guided login:

```bash
# Claude / Anthropic OAuth, stored in ~/.archon/.credentials.json
archon auth login --provider anthropic

# OpenAI Codex OAuth, stored beside the Anthropic token
# (Archon also detects an existing official Codex CLI login)
archon auth login --provider openai-codex

# Inspect both without printing secrets
archon auth status

# Google Gemini API key for docs VLM
archon auth login --provider google

# Use Codex explicitly for one-shot chat
archon chat --provider openai-codex "summarize this repository"
```

To make the full interactive TUI use Codex, set:

```toml
[llm]
provider = "openai-codex"

[api]
default_model = "gpt-5.4" # optional; Archon uses this automatically if the old default is Claude-shaped
```

In Codex TUI sessions, the bottom status bar is initialized from the active Codex model, and `/model` accepts Codex shortcuts and model IDs such as `default`, `codex`, `mini`, `gpt-5.5`, `gpt-5.4`, `gpt-5.4-mini`, and `gpt-5.3-codex`. Anthropic sessions continue to validate Claude shortcuts and Claude model IDs.

Anthropic OAuth requests use the same Claude Code identity-spoof path as the agent and pipeline runners. Codex OAuth requests use the OpenAI Codex provider for chat, TUI sessions, tool use, subagents, `/btw`, team runs, and provider-neutral pipelines when `[llm].provider = "openai-codex"`. API-key users can set `ANTHROPIC_API_KEY=sk-ant-api...`; proxy users can still point the Anthropic-compatible URL at OpenRouter, DeepSeek, LiteLLM, or another compatible endpoint and use native/API-key mode.

## Video Evidence

Video evidence is ingested through `archon video` and becomes ordinary document
chunks, so `archon docs search`, `archon docs answer`, and `archon kb process`
consume it without a separate flag.

```bash
# Local video with a user transcript
archon video ingest ./lecture.mp4 --transcript ./lecture.vtt --frames none
archon video inspect <video-id>

# YouTube URL with your own transcript, no media download
archon video ingest "https://www.youtube.com/watch?v=abc123" \
  --transcript ./talk.vtt \
  --metadata-only

# YouTube URL with governed local download and whisper-cpp ASR
archon video ingest "https://youtu.be/abc123" --frames hybrid --asr whisper-cpp --yes

# Add YouTube evidence to a named KB bucket
archon video ingest "https://youtu.be/abc123" --kb trading-elliott-wave --frames hybrid --asr whisper-cpp --yes
archon kb process --kb trading-elliott-wave --claims --entities --relations

# Frame extraction for charts, diagrams, and slides
archon video ingest ./market-review.mp4 --frames hybrid --vlm --yes
archon video transcript <video-id> --format vtt
archon docs answer "what did the chart show?"
```

Answers cite video chunks as `video@MM:SS` when timestamp provenance is present.
When policy allows caption capture, Archon tries YouTube captions before ASR;
frame OCR can use local RapidOCR/OpenCV fallbacks for chart-heavy videos.
See [`docs/video.md`](docs/video.md) for ASR, OCR/VLM, policy, and compliance
details.

## Documentation map

The docs are organised by user goal:

| Section | Find this here |
|---|---|
| **Getting started** | [`docs/getting-started/`](docs/getting-started/) — install, first run, quick start |
| **Architecture** | [`docs/architecture/`](docs/architecture/) — overview, learning systems, pipelines, Evidence Engine diagrams |
| **Evidence Engine** | [`docs/evidence-engine.md`](docs/evidence-engine.md) — documents, KB, provenance, game theory, completion integrity, governed learning |
| **Trading Lab** | [`docs/trading-lab.md`](docs/trading-lab.md) and [`docs/cookbook/trading-lab.md`](docs/cookbook/trading-lab.md) — governed trading research, strategy specs, Pine prototypes, deterministic backtests, TradingView replay-paper evidence, workflow specs, paper/live gates, and risk/audit controls |
| **Providers** | [`docs/providers/`](docs/providers/) — provider runtime, Codex app-server, Claude Code spoofing, auth profiles, cloud/local providers |
| **Agents & learning** | [`docs/agents/`](docs/agents/) and [`docs/learning/`](docs/learning/) — governed agent evolution, memory promotion, permission governance |
| **World model** | [`docs/architecture/world-model.md`](docs/architecture/world-model.md) — local trace corpus, advisory predictions, training backends, retention |
| **Reasoning quality** | [`docs/architecture/reasoning-quality.md`](docs/architecture/reasoning-quality.md) — visible claim/evidence events, correction links, critic policy, proactive briefing |
| **Security** | [`docs/security/`](docs/security/) — tool preflight, sandboxing, Docker, SSH, OpenShell |
| **Reference** | [`docs/reference/`](docs/reference/) — slash commands, tools, skills, permissions, config schema, CLI flags, env vars |
| **Integrations** | [`docs/integrations/`](docs/integrations/) — MCP, plugins, hooks, identity spoofing, VLM image descriptions, LSP, IDE extensions |
| **Cookbook** | [`docs/cookbook/`](docs/cookbook/) — real-world evidence workflows, YouTube/video evidence, strategic engagement, memory-driven coding, pipeline rewind, god-code pipeline, custom agents |
| **Operations** | [`docs/operations/`](docs/operations/) — sessions, web workbench, TUI, cost, compaction, cron, remote control, troubleshooting, data locations |
| **Development** | [`docs/development/`](docs/development/) — contributing, dev flow gates, adding tools/skills/agents, release process |
| **Release notes** | [`docs/release-notes/`](docs/release-notes/) — per-version changelogs |

## Repository structure

```
archon-cli/
├── src/                       # binary entry point + CLI layer
├── crates/                    # 26-crate workspace
│   ├── archon-cli-workspace/  # binary
│   ├── archon-tui/            # ratatui terminal UI
│   ├── archon-core/           # agent loop, tools, skills
│   ├── archon-consciousness/  # rules, personality, persistence
│   ├── archon-session/        # session checkpoints (CozoDB)
│   ├── archon-memory/         # memory graph + embeddings (CozoDB)
│   ├── archon-llm/            # provider clients + identity/spoofing
│   ├── archon-tools/          # 65 registered tools
│   ├── archon-permissions/    # 7 permission modes
│   ├── archon-mcp/            # MCP transport
│   ├── archon-pipeline/       # 50+46 agent pipelines + game theory + learning systems
│   ├── archon-docs/           # document intelligence, OCR, retrieval
│   ├── archon-knowledge/      # claims, entities, contradictions
│   ├── archon-provenance/     # chain hashes, W3C PROV export
│   ├── archon-completion/     # completion integrity and trust
│   ├── archon-learning/       # governed learning events/manifests
│   ├── archon-meaning/        # labels, contrastive pairs, triplets
│   ├── archon-constellation/  # centroids, scoring, drift
│   ├── archon-world-model/    # local trace world model, advisor, counterfactual scoring
│   ├── archon-reasoning-quality/ # visible claim/evidence event store
│   ├── archon-policy/         # policy gates
│   ├── archon-leann/          # semantic code search
│   ├── archon-plugin/         # dynamic plugin loading
│   ├── archon-sdk/            # embedding API + IDE bridge
│   ├── archon-context/        # context compaction
│   ├── archon-observability/  # metrics, tracing
│   └── ...
├── docs/                      # user-facing documentation
└── scripts/                   # dev flow gates, helpers
```

## Status

- Current version: **v1.6.0** ([release notes](docs/release-notes/v1.6.0.md))
- v1.6.0 lets agents hand work to each other: a durable task board with atomic claims that survive the memory garden, subagent identity so a write can be attributed, a self-check that reports a file over the line limit back into the agent's own transcript, and a browser view of the board and of what is running. Also fixes consolidation silently reporting an unavailable semantic pass as a clean store on any second archon instance. **One default changed**: the repository code index no longer builds at session start — it was costing roughly seventeen CPU-hours on a 3,200-file repository, on sessions that never asked for code context. Set `[code_index] index_on_startup = true` to restore it.
- Stable release for local world-model advisory learning, first-class reasoning-quality events, provider runtime governance, Cozo-backed agent evolution, permission preflight, and sandbox routing
- v1.5.0 gives every run one graph representation: the `TaskGraph` IR, an ambient trace, and synchronous guardrail admission. On top of it, `archon requirements trace` reports a four-level proof ladder (with `--falsify` to check a verifier actually depends on the code it claims to prove) and `archon workflow lint` runs four advisory analyses that can never fail a run. Adds the `/workflow-prd` and `/workflow-prd-spec` skills, and lets SONA tune the four generated-workflow limits plus the implementation-wave fan-out width, all fail-closed below five observations. **Two behaviour changes:** hooks now resolve a POSIX shell on Windows, and `ARCHON_DATA_DIR` means one directory.
- v1.4.0 makes the JEPA world model actually learn: trainable encoders with stop-gradient and a live collapse gate, dense embedding input in place of hashed excerpt text, a pre-turn advisory hook in the live agent path, and an automatic candidate evaluate/shadow/promote/rollback lifecycle. Also adds the `archon draft` FCDP pipeline with provenance import.
- v1.3.11 adds the governed Trading Lab substrate: strategy specs, Pine prototypes, data/backtest/paper/live gates, risk/audit controls, learning hooks, and detailed user/cookbook documentation.
- v1.3.10 adds provider-neutral dynamic workflows: `/workflow` planning/running/resume controls, durable workflow bundles, live TUI activity rows, web workflow inspection, learning ledgers, sanitized events, reusable templates, and workflow docs.
- v1.3.9 moves document embeddings out of the hot Cozo write path into a RocksDB raw-vector store, adds resumable legacy-vector migration, Rust-HNSW compaction, durable index-queue/daemon controls, and clearer vector-status diagnostics.
- v1.3.8 adds the Cognitive Executive Loop: situation classification, candidate planning, policy-gated JEPA/world-model scoring, compact decision/reflection ledgers, autonomous cognitive ticks, an opt-in Rust daemon, CLI/TUI/web executive-state inspection, and Opus-tier alias support for `claude-opus-4-8`.
- v1.3.7 adds policy-gated autonomous governed learning via `archon learning tick`, provider-resolved pipeline subagent activity, and refreshed self-learning documentation.
- v1.3.6 adds governed video evidence capture fallbacks: caption-first YouTube ingest, frame-friendly `yt-dlp` format selection, optional OpenCV frame extraction fallback, and optional RapidOCR image/frame OCR.
- v1.3.5 hardens governed video evidence ingest: YouTube acquisition feeds local media into ASR/frame paths, `whisper-cpp` JSON segments become timecoded chunks, frame extraction uses PNG output, and setup/docs cover video dependencies across supported OS families.
- v1.3.4 adds DeepSeek provider parity for Anthropic-compatible sessions, generic non-Claude `/model` selection, DeepSeek context/catalog coverage, and provider-aware TUI/session cost display.
- v1.3.3 adds governed video evidence ingest, transcript/frame evidence storage, `/video` parity, and policy-gated video summaries.
- v1.3.2 finalizes the JEPA eval pipeline with quick/full modes, promotion-safe eval provenance, eval-run status commands, backend/cache deferral warnings, and source-size cleanup.
- v1.3.1 adds JEPA world-model training/runtime, normal-session and pipeline guardrails, CUDA/MLX accelerator support, and fresh-setup cookbook docs.
- v1.3.0 removes model-facing `max_turns`, hardens compaction persistence/request-pressure recovery, moves long-tail agent discovery behind `AgentCatalog`, and preserves accurate subagent/context status.
- v1.2.9 corrects the autocompaction trigger to use the current message-list estimate (not cumulative token usage) and converts proactive compaction failures to a soft-fail that falls through to the same turn's normal provider call. Reactive compaction remains fatal.
- v1.2.8 completes the auto-compaction PRD: provider-backed summaries, per-provider context windows, prompt budgeting, context warning/source UI, and provider-aware prompt-cache policy.

## Contributing

See [`docs/development/contributing.md`](docs/development/contributing.md). Every task passes the 6-gate dev flow ([`docs/development/dev-flow-gates.md`](docs/development/dev-flow-gates.md)) before merge.

## License

See [`LICENSE`](LICENSE) (MIT). Exception: the [`plugins/`](plugins/README.md) collection is derived from Apache-2.0 sources and remains Apache-2.0 — see [its licensing section](plugins/README.md#licensing).

archon-cli can proxy Anthropic Claude and OpenAI Codex-compatible APIs. You must have valid credentials or an active subscription and comply with the relevant provider usage policies.
