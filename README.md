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

**Current release: v1.8.0** — [release notes](docs/release-notes/v1.8.0.md)

The cognitive metrics v1.7.0 started producing now have a consumer that can say
no: release gates judged per cohort, blocking the agent's own improvement
proposals when a segment degrades. The self-model writes its prediction before
the action it predicts, and reflections are carried into later turns and measured
for reuse rather than written and forgotten. The learning roadmap's R0 entry gate
is a script that can go red instead of a paragraph, wired into CI. The corpus
viewer renders PDFs, sandboxed. And workflow runs finally appear on the task
board — the third dispatch path that had to be wired separately.

Built on v1.7.0, which wired four subsystems that had been built, tested and
never called, and gave the knowledge base `docs compile`, `docs answer` and
`docs export`.

> **On upgrade: project capability manifests no longer hoist tools**, so a task
> is no longer obliged to invoke every tool in `.archon/project.json`; env keys
> still hoist, and tools already on disk are reported as inert rather than
> deleted. **A declared artifact must now be a regular, non-empty file** — a
> directory or a zero-byte file used to satisfy the contract. `events.jsonl`
> gains a `blocking_gap_detected` kind for monitors to filter on, and garden
> review-band adjudication now runs detached after startup instead of blocking
> the session bootstrap. See the
> [release notes](docs/release-notes/v1.8.0.md#upgrade-notes).
>
> From v1.7.0: the cognitive schema migrates on first open, and history for two
> `cognitive_tick_audit` columns is set to null — under the old code those were
> hardcoded, so every stored value was a fabrication rather than a measurement.
>
> From v1.6.0: the repository code index no longer builds at
> session start — it cost roughly seventeen CPU-hours on a 3,200-file repository,
> on sessions that never asked for code context. Restore it with
> `[code_index] index_on_startup = true`. And Intel macOS is no longer built:
> ONNX Runtime stopped publishing x64 macOS binaries and the Rust `ort` bindings
> followed, so the embedding runtime cannot be linked there. Apple Silicon covers
> every Mac still receiving macOS updates.

Every release from v0.1.6 onward is indexed with a one-line summary in the
[documentation map](docs/README.md#release-notes). This section carries the
current release and what changes on upgrade — nothing a reader can look up.

## Contributing

See [`docs/development/contributing.md`](docs/development/contributing.md). Every task passes the 6-gate dev flow ([`docs/development/dev-flow-gates.md`](docs/development/dev-flow-gates.md)) before merge.

## License

See [`LICENSE`](LICENSE) (MIT). Exception: the [`plugins/`](plugins/README.md) collection is derived from Apache-2.0 sources and remains Apache-2.0 — see [its licensing section](plugins/README.md#licensing).

archon-cli can proxy Anthropic Claude and OpenAI Codex-compatible APIs. You must have valid credentials or an active subscription and comply with the relevant provider usage policies.
