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
| **Engineering practice** | [`docs/defensive-patterns.md`](docs/defensive-patterns.md) — rules for writing checks that cannot lie · [`docs/postmortem/`](docs/postmortem/README.md) — numbered incident writeups · [`docs/decisions/`](docs/decisions/README.md) — decision records, including the `rejected` bucket |
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

**Current release: v1.9.3** — [release notes](docs/release-notes/v1.9.3.md)

Plan Mode now has a durable approval lifecycle: explicit user exit controls,
safe permission restoration, editable plan documents, plan-linked task
materialization, authoritative completion evidence, reconciliation, and a
Plan-Mode-specific model override. The trust boundary blocks subagents from entering
or exiting Plan Mode and fails closed when evidence or approval authority cannot
be verified.

Registered canonical skills now appear in slash autocomplete. Primary commands
remain first and keep dispatch precedence; rows are labelled `[command]` or
`[skill]`, aliases are not duplicated, and shadowed skills are omitted with an
exact startup warning.

Built on v1.9.0, where a knowledge base stopped being two disjoint things. `archon kb kbs` and the web
Ingest tab list the same union, so a name created on either surface is visible
and usable from both — and a name you did not write down is recoverable at all.
Corrections only reinforce a rule once something has been shown to have caused
them, with the attribution itself running in shadow. The Memory Garden runs
under a single-run lock and a work budget, and its irreversible operations
became proposals a human approves. Replay weights transitions by latent
surprise, which was recorded and never read.

The subagent runtime stopped repeating itself: five whole-array copies per round
became two, a transcript opens once instead of once per message, and a ten-way
fan-out queries the memory store three times rather than thirty. Typing is
multi-line — Shift+Enter inserts a newline where the terminal can express it —
and the input area no longer leaves glyphs behind when it grows and shrinks.

Built on v1.8.0, which gave the cognitive metrics a consumer that can say no,
and v1.7.0, which wired four subsystems that had been built, tested and never
called.

> **On upgrade: a correction no longer reinforces a rule unless something has
> been shown to have caused it.** Attribution runs in shadow and fails closed,
> so an unattributable correction is recorded and reinforces nothing — the
> extractor's deferred semantic pass reinforces nothing at all now, because it
> records against an action window that has already moved. **Scheduled garden
> consolidation never deletes**: staleness and overflow pruning become proposals
> you approve through `/garden proposals`, while `/garden` run by hand behaves
> as before. The web workbench answers **403 rather than 200** when policy
> refuses an action, so a caller checking only the status code no longer reads a
> refusal as success. Enter still submits; **Shift+Enter inserts a newline**, and
> Ctrl+L forces a redraw. See the
> [release notes](docs/release-notes/v1.9.0.md#upgrade-notes).
>
> From v1.9.3: **a write to a file the agent has not read is now refused.**
> `Edit`, `Write` and `NotebookEdit` are checked against what `Read`, `Grep` and
> `NotebookRead` recorded; `Bash` is not checked. Set
> `[filesystem].read_before_edit` to `"warn"` or `"off"` to soften or disable it.
> **`voice.hotkey` defaults to `"ctrl+v"`**, which is the key that has always
> been bound. **Building on Linux needs `libasound2-dev`** now that microphone
> capture is a default feature. `/feedback` rates a message rather than
> submitting a report, replacing a skill that recorded nothing. See the
> [release notes](docs/release-notes/v1.9.3.md#upgrade-notes).
>
> From v1.9.2: **a structured `ExitPlanMode` submission now requires
> approval**, while `/plan off`, `/plan exit`, and `/plan done` remain explicit
> direct exits to `default`. Only plan-linked task rows persist and rehydrate;
> unrelated manual tasks remain process-scoped. Skills are discovered when a
> session starts, so restart Archon after adding or editing a `SKILL.md`. See the
> [release notes](docs/release-notes/v1.9.2.md#upgrade-notes).
>
> From v1.9.1: **cost figures move in both directions** — cache reads were
> priced at zero and writes at par, so caching could only ever look like a
> saving. `prompt_cache_conversation = false` no longer disables the tools and
> system checkpoints along with the message one. This turn's volatile system
> blocks move onto the last user message (`prompt_cache_reorder`, default on),
> which changes where the model sees them. `RUST_LOG` now reaches the session
> log. See the [release notes](docs/release-notes/v1.9.1.md#upgrade-notes).
>
> From v1.8.0: project capability manifests no longer hoist tools, a declared
> artifact must be a regular non-empty file, and `events.jsonl` gained a
> `blocking_gap_detected` kind.
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

Before writing a gate, a lint, a CI step, or a test involving a subprocess, a platform difference, or a clock, read [`docs/defensive-patterns.md`](docs/defensive-patterns.md). Every rule there is traced to a [postmortem](docs/postmortem/README.md) of a check in this repo that reported green while inspecting nothing.

## License

See [`LICENSE`](LICENSE) (MIT). Exception: the [`plugins/`](plugins/README.md) collection is derived from Apache-2.0 sources and remains Apache-2.0 — see [its licensing section](plugins/README.md#licensing).

archon-cli can proxy Anthropic Claude and OpenAI Codex-compatible APIs. You must have valid credentials or an active subscription and comply with the relevant provider usage policies.
