# archon-cli

<div align="center">
  <img src="archon-avatar.png" alt="Archon Avatar" width="600" />
</div>

A strategic engineering CLI built in Rust. Self-learning agent platform with
persistent memory, multi-agent pipelines, Evidence Engine provenance, document
intelligence, governed learning, and identity-aware Anthropic/Codex provider
integration.

> **Documentation has moved.** This README is now a landing page. The full structured docs live in [`docs/`](docs/README.md) — start there.

---

## Quick start

```bash
# Build (Rust 1.85+, edition 2024)
git clone https://github.com/ste-bah/archon-cli
cd archon-cli
cargo build --release --bin archon

# Authenticate with Claude/Anthropic OAuth or API-key billing
./target/release/archon auth login --provider anthropic
# or: export ANTHROPIC_API_KEY="sk-ant-api..."

# Optional: authenticate with a ChatGPT/Codex subscription
./target/release/archon auth login --provider openai-codex
./target/release/archon auth status

# Optional: store a Google Gemini API key for cloud VLM image descriptions
./target/release/archon auth login --provider google

# Run interactive TUI
./target/release/archon

# Non-interactive print mode
./target/release/archon -p "summarize Cargo.toml" --output-format json
```

WSL2 builders: add `-j1` to avoid OOM during compilation.

Full installation guide: [`docs/getting-started/installation.md`](docs/getting-started/installation.md).

## What archon-cli is

| | claude-code (TS/Bun) | archon-cli (Rust) |
|---|---|---|
| Runtime | TypeScript / Bun | Rust 1.85+ |
| Memory | markdown files | CozoDB graph + embeddings |
| Documents | ad hoc file reads | OCR, image ingest, chunks, embeddings, hybrid retrieval, citations |
| Pipelines | Single-agent loop | 50-agent coding + 46-agent research + 84-specialist game theory |
| Reasoning | Direct LLM call | 12 reasoning modes (deductive, inductive, abductive, analogical, adversarial, counterfactual, temporal, constraint, decomposition, first-principles, causal, contextual) |
| Learning | None | 8 subsystems: SONA, ReasoningBank, GNN, CausalMemory, Provenance, DESC, Reflexion, AutoCapture |
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

Anthropic OAuth requests use the same Claude Code identity-spoof path as the agent and pipeline runners. Codex OAuth requests use the OpenAI Codex provider for chat, TUI sessions, tool use, subagents, `/btw`, team runs, and provider-neutral pipelines when `[llm].provider = "openai-codex"`. API-key users can set `ANTHROPIC_API_KEY=sk-ant-api...`; proxy users can still point the Anthropic-compatible URL at OpenRouter, DeepSeek, LiteLLM, or another compatible endpoint and use native/API-key mode.

## Documentation map

The docs are organised by user goal:

| Section | Find this here |
|---|---|
| **Getting started** | [`docs/getting-started/`](docs/getting-started/) — install, first run, quick start |
| **Architecture** | [`docs/architecture/`](docs/architecture/) — overview, learning systems, pipelines, Evidence Engine diagrams |
| **Evidence Engine** | [`docs/evidence-engine.md`](docs/evidence-engine.md) — documents, KB, provenance, game theory, completion integrity, governed learning |
| **Providers** | [`docs/providers/`](docs/providers/) — provider runtime, Codex app-server, Claude Code spoofing, auth profiles, cloud/local providers |
| **Agents & learning** | [`docs/agents/`](docs/agents/) and [`docs/learning/`](docs/learning/) — governed agent evolution, memory promotion, permission governance |
| **Security** | [`docs/security/`](docs/security/) — tool preflight, sandboxing, Docker, SSH, OpenShell |
| **Reference** | [`docs/reference/`](docs/reference/) — slash commands, tools, skills, permissions, config schema, CLI flags, env vars |
| **Integrations** | [`docs/integrations/`](docs/integrations/) — MCP, plugins, hooks, identity spoofing, VLM image descriptions, LSP, IDE extensions |
| **Cookbook** | [`docs/cookbook/`](docs/cookbook/) — real-world evidence workflows, strategic engagement, memory-driven coding, god-code pipeline, custom agents |
| **Operations** | [`docs/operations/`](docs/operations/) — sessions, TUI, cost, compaction, cron, remote control, troubleshooting, data locations |
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
│   ├── archon-tools/          # 43 built-in tools
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

- Current version: **v1.1.0-beta.1** ([release notes](docs/release-notes/v1.1.0-beta.1.md))
- Beta release for provider runtime governance, Cozo-backed agent evolution, permission preflight, and sandbox routing
- v1.1.0-beta.1 adds Codex app-server support, Claude Code spoof compatibility coverage, durable provider telemetry, governed profile evolution, and Docker/SSH/OpenShell sandbox documentation

## Contributing

See [`docs/development/contributing.md`](docs/development/contributing.md). Every task passes the 6-gate dev flow ([`docs/development/dev-flow-gates.md`](docs/development/dev-flow-gates.md)) before merge.

## License

See [`LICENSE`](LICENSE).

archon-cli can proxy Anthropic Claude and OpenAI Codex-compatible APIs. You must have valid credentials or an active subscription and comply with the relevant provider usage policies.
