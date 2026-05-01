# archon-cli

<div align="center">
  <img src="archon-avatar.png" alt="Archon Avatar" width="600" />
</div>

A strategic engineering CLI built in Rust. Self-learning agent platform with persistent memory, multi-agent pipelines, and identity-aware Anthropic API integration.

> **Documentation has moved.** This README is now a landing page. The full structured docs live in [`docs/`](docs/README.md) — start there.

---

## Quick start

```bash
# Build (Rust 1.85+, edition 2024)
git clone https://github.com/ste-bah/archon-cli
cd archon-cli
cargo build --release --bin archon

# Authenticate
export ANTHROPIC_API_KEY="sk-ant-..."
# or: ./target/release/archon login

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
| Pipelines | Single-agent loop | 50-agent coding + 46-agent research |
| Reasoning | Direct LLM call | 12 reasoning modes (deductive, inductive, abductive, analogical, adversarial, counterfactual, temporal, constraint, decomposition, first-principles, causal, contextual) |
| Learning | None | 8 subsystems: SONA, ReasoningBank, GNN, CausalMemory, Provenance, DESC, Reflexion, AutoCapture |
| Identity | Native | Spoof (Claude Code mimicry) or native |

archon-cli is **not affiliated with Anthropic**. It uses the Anthropic Claude API and requires a valid API key or Claude.ai subscription.

## Documentation map

The docs are organised by user goal:

| Section | Find this here |
|---|---|
| **Getting started** | [`docs/getting-started/`](docs/getting-started/) — install, first run, quick start |
| **Architecture** | [`docs/architecture/`](docs/architecture/) — overview, learning systems, pipelines |
| **Reference** | [`docs/reference/`](docs/reference/) — slash commands (65), tools (43), skills (55), permissions (7 modes), config schema, CLI flags, env vars |
| **Integrations** | [`docs/integrations/`](docs/integrations/) — MCP, plugins, hooks, identity spoofing, LSP, IDE extensions |
| **Cookbook** | [`docs/cookbook/`](docs/cookbook/) — strategic engagement, memory-driven coding, god-code pipeline, custom agents |
| **Operations** | [`docs/operations/`](docs/operations/) — sessions, TUI, cost, compaction, cron, remote control, troubleshooting, data locations |
| **Development** | [`docs/development/`](docs/development/) — contributing, dev flow gates, adding tools/skills/agents, release process |
| **Release notes** | [`docs/release-notes/`](docs/release-notes/) — per-version changelogs |

## Repository structure

```
archon-cli/
├── src/                       # binary entry point + CLI layer
├── crates/                    # 21-crate workspace
│   ├── archon-cli-workspace/  # binary
│   ├── archon-tui/            # ratatui terminal UI
│   ├── archon-core/           # agent loop, tools, skills
│   ├── archon-consciousness/  # rules, personality, persistence
│   ├── archon-session/        # session checkpoints (CozoDB)
│   ├── archon-memory/         # memory graph + embeddings (CozoDB)
│   ├── archon-llm/            # Anthropic API client + spoofing
│   ├── archon-tools/          # 43 built-in tools
│   ├── archon-permissions/    # 7 permission modes
│   ├── archon-mcp/            # MCP transport
│   ├── archon-pipeline/       # 50+46 agent pipelines + learning systems
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

- Current version: **v0.1.35** ([release notes](docs/release-notes/v0.1.35.md))
- Active development; pre-1.0 means breaking changes can land in minor versions
- Phase 6 complete (pipelines + learning systems wired); see release notes for the v0.1.6 → v0.1.35 stabilisation arc

## Contributing

See [`docs/development/contributing.md`](docs/development/contributing.md). Every task passes the 6-gate dev flow ([`docs/development/dev-flow-gates.md`](docs/development/dev-flow-gates.md)) before merge.

## License

See [`LICENSE`](LICENSE).

archon-cli proxies the Anthropic Claude API. You must have a valid API key or active subscription and comply with Anthropic's usage policies.
