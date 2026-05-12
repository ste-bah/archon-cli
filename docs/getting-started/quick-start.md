# Quick start

5 minutes from clone to running agent.

## Prerequisites

- Rust 1.85+ (edition 2024)
- Either an Anthropic API key, an active Claude.ai subscription, or Codex OAuth with `[llm].provider = "openai-codex"`

If you don't have Rust:
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

## Build

```bash
git clone https://github.com/ste-bah/archon-cli
cd archon-cli
sudo scripts/install-system-deps.sh --check || sudo scripts/install-system-deps.sh
cargo build --release --bin archon
ARCHON_BIN="$(pwd)/target/release/archon"
```

Build time: ~3-4 min on a modern laptop. On WSL2, add `-j1` to avoid OOM:
```bash
cargo build --release --bin archon -j1
```

The binary lands at `target/release/archon` (~66 MB).

## Initialise project

For a new, empty project directory:

```bash
mkdir -p ~/projects/my-archon-project
sh scripts/archon-init.sh \
  --target ~/projects/my-archon-project \
  --archon-cli-repo "$(pwd)"
cd ~/projects/my-archon-project
```

This creates `.archon/`, `prds/`, and `tasks/` directories and a `.gitignore`
entry in the project. Safe to re-run — always idempotent. `archon-init.sh`
expects the target directory to exist; use `mkdir -p` first for a brand-new
path.

## Authenticate

Pick one:

```bash
# Option A: API key
export ANTHROPIC_API_KEY="sk-ant-..."

# Option B: Anthropic OAuth (Claude subscriber)
"$ARCHON_BIN" auth login --provider anthropic

# Option C: Codex OAuth (ChatGPT/Codex subscriber)
"$ARCHON_BIN" auth login --provider openai-codex
```

## First run

```bash
"$ARCHON_BIN"
```

That opens the TUI. Type `/help` for the command list, or `/setup-archon-skills` for an interactive configuration wizard.

## Web workbench

The web workbench is embedded into the `archon` binary. Normal users do not
need Node.js or a separate frontend install. Launch it from the project root so
the browser UI inspects that project's `.archon/`, docs corpus, memory,
pipelines, and world-model state:

```bash
"$ARCHON_BIN" web --port 8421 --bind-address 127.0.0.1
```

This opens `http://localhost:8421` by default. Use `--no-open` when running on
a remote box or in WSL where you want to open the URL manually.

## Smoke test

Non-interactive print mode, useful for CI:
```bash
"$ARCHON_BIN" -p "summarize this project layout" --output-format json
```

If you see structured JSON output, the install is healthy.

## Next steps

- [Installation](installation.md) — full build details for every OS, OS-specific dependencies, common build problems
- [First run](first-run.md) — what data archon writes to disk, where logs go, common gotchas
- [Web workbench](../operations/web-workbench.md) — browser tabs, data sources, action safety, and setup
- [Slash commands reference](../reference/slash-commands.md) — the 78 primary commands
- [Sandboxing](../security/sandboxing.md) — optional Docker, SSH, and OpenShell isolation
- [Cookbook](../cookbook/) — task-oriented walkthroughs
