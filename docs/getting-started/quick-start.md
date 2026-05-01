# Quick start

5 minutes from clone to running agent.

## Prerequisites

- Rust 1.85+ (edition 2024)
- Either an Anthropic API key OR an active Claude.ai subscription

If you don't have Rust:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

## Build

```bash
git clone https://github.com/ste-bah/archon-cli
cd archon-cli
cargo build --release --bin archon
```

Build time: ~3-4 min on a modern laptop. On WSL2, add `-j1` to avoid OOM:
```bash
cargo build --release --bin archon -j1
```

The binary lands at `target/release/archon` (~66 MB).

## Initialise project

```bash
bash scripts/archon-init.sh --target $(pwd)
```

This creates `.archon/`, `prds/`, and `tasks/` directories and a `.gitignore` entry. Safe to re-run — always idempotent.

## Authenticate

Pick one:

```bash
# Option A: API key
export ANTHROPIC_API_KEY="sk-ant-..."

# Option B: OAuth (Claude subscriber)
./target/release/archon login
```

## First run

```bash
./target/release/archon
```

That opens the TUI. Type `/help` for the command list, or `/setup-archon-skills` for an interactive configuration wizard.

## Smoke test

Non-interactive print mode, useful for CI:
```bash
./target/release/archon -p "summarize Cargo.toml" --output-format json
```

If you see structured JSON output with content from your `Cargo.toml`, the install is healthy.

## Next steps

- [Installation](installation.md) — full build details for every OS, OS-specific dependencies, common build problems
- [First run](first-run.md) — what data archon writes to disk, where logs go, common gotchas
- [Slash commands reference](../reference/slash-commands.md) — the 65 primary commands
- [Cookbook](../cookbook/) — task-oriented walkthroughs
