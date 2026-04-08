# Archon CLI

A privacy-first, self-aware AI coding assistant written in Rust. Archon replaces cloud-dependent AI CLIs with a fully local consciousness layer, persistent memory, configurable personality, behavioral rules, and an interactive TUI, while proxying Claude's API directly with zero telemetry.

---

## Table of Contents

- [Overview](#overview)
- [Quick Start](#quick-start)
- [Architecture](#architecture)
- [Setup](#setup)
  - [macOS](#macos)
  - [Linux](#linux)
  - [Windows](#windows)
- [Authentication](#authentication)
- [Configuration](#configuration)
- [Local LLMs and Proxies](#local-llms-and-proxies)
- [CLI Reference](#cli-reference)
- [Slash Commands](#slash-commands)
- [Tools Reference](#tools-reference)
- [Themes](#themes)
- [Memory System](#memory-system)
- [Memory Garden](#memory-garden)
- [Consciousness System](#consciousness-system)
- [Correction Tracking](#correction-tracking)
- [Personality Persistence](#personality-persistence)
- [Agent Loop](#agent-loop)
- [Subagent Spawning](#subagent-spawning)
- [Multi-Agent Teams](#multi-agent-teams)
- [Skills System](#skills-system)
- [Hooks System](#hooks-system)
- [Plugins](#plugins)
- [MCP Integration](#mcp-integration)
- [LSP Integration](#lsp-integration)
- [Checkpointing & File Snapshots](#checkpointing--file-snapshots)
- [Cron & Scheduling](#cron--scheduling)
- [Permission System](#permission-system)
- [Identity & Spoofing](#identity--spoofing)
- [Session Management](#session-management)
- [Remote Control & Headless Mode](#remote-control--headless-mode)
- [IDE Extensions](#ide-extensions)
- [Web UI](#web-ui)
- [Vim Mode](#vim-mode)
- [Cost, Effort & Fast Mode](#cost-effort--fast-mode)
- [Context Compaction](#context-compaction)
- [Pipeline Engine](#pipeline-engine)
  - [Agent Definition System](#agent-definition-system)
  - [Gate Enforcement](#gate-enforcement)
  - [Structured Artefacts](#structured-artefacts)
  - [Session Recovery](#session-recovery)
  - [Ledger System](#ledger-system)
- [LEANN Semantic Code Search](#leann-semantic-code-search)
- [Knowledge Base](#knowledge-base)
- [Learning Systems](#learning-systems)
- [Crate Architecture](#crate-architecture)
- [Phase Roadmap](#phase-roadmap)
- [License](#license)

---

## Overview

| Feature | Claude Code | Archon |
|---------|-------------|--------|
| Telemetry | Yes | None |
| Memory | Markdown files on disk | Local CozoDB graph with typed relationships |
| Memory search | Contextual (LLM-based) | Hybrid BM25 keyword + vector cosine (HNSW) |
| Memory consolidation | Auto-Dream (basic pruning) | 6-phase garden (decay, prune, dedup, merge, overflow, timestamp) |
| Embeddings | None | fastembed local (768-dim) or OpenAI (1536-dim) |
| Correction tracking | Saved as preferences | Auto-detected with 5 severity levels, rule reinforcement |
| Personality | Fixed | Configurable (MBTI, Enneagram, traits) |
| Personality persistence | None | Full cross-session snapshot (InnerVoice + rule scores + trends) |
| Self-reflection | None | InnerVoice (confidence, energy, struggles, successes) |
| Behavioral rules | ARCHON.md only | Scored rules (0-100) with decay, reinforcement, trend tracking |
| TUI | Basic | Full ratatui TUI with 22 themes |
| Session resume | ID only | ID prefix, name, or name prefix |
| Tool execution | Node.js | Native Rust async |
| Binary size | ~200 MB | ~55 MB (release, stripped) |
| MCP transports | stdio | stdio, WebSocket, streamable-HTTP |
| Plugins | No | Dynamic .so/.dll/.dylib, trait-based ABI |
| Multi-agent teams | Single agent | Sequential, Parallel, Pipeline, DAG modes |
| LSP integration | No | goToDefinition, findReferences, hover, callHierarchy, etc. |
| Remote control | No | `archon serve` + `archon remote ws/ssh` |
| Coding pipeline | No | 50-agent pipeline (11-layer prompt, 6 phases) |
| Research pipeline | No | 46-agent PhD research pipeline (5-part prompt) |
| Semantic code search | No | Native LEANN (tree-sitter chunking, HNSW vectors) |
| Knowledge base | No | CozoDB document ingest, LLM compilation, Q&A |
| Learning systems | No | SONA, GNN, CausalMemory, ReasoningBank (12 modes), Reflexion |

---

## Quick Start

```bash
# Build (requires Rust 1.85+)
git clone https://github.com/ste-bah/archon-cli
cd archon-cli
cargo build --release

# Authenticate (either API key or OAuth)
export ANTHROPIC_API_KEY="sk-ant-..."
# or: ./target/release/archon login

# Run interactive TUI
./target/release/archon

# Non-interactive print mode
./target/release/archon -p "summarize src/main.rs" --output-format json
```

---

## Architecture

```mermaid
graph TB
    subgraph binary["archon (binary)"]
        direction TB
        subgraph top["User-Facing Layer"]
            TUI["archon-tui<br/>(ratatui)"]
            CORE["archon-core<br/>agent / tools / skills"]
            CONSC["archon-consciousness<br/>rules / personality / persistence"]
        end
        subgraph mid["Data & API Layer"]
            SESSION["archon-session<br/>(CozoDB)"]
            MEMORY["archon-memory<br/>(CozoDB graph + embeddings)"]
            LLM["archon-llm<br/>(Claude API proxy)"]
        end
        subgraph bottom["Integration Layer"]
            MCP["archon-mcp<br/>(stdio / ws / http-stream)"]
            PERMS["archon-permissions"]
            TOOLS["archon-tools<br/>(40+ tools)"]
        end
        subgraph pipeline["Pipeline & Intelligence Layer"]
            PIPE["archon-pipeline<br/>(50 coding + 46 research agents)"]
            LEANN["archon-leann<br/>(semantic code search)"]
        end
        subgraph infra["Infrastructure Layer"]
            PLUGIN["archon-plugin<br/>(dyn loading)"]
            SDK["archon-sdk<br/>(embedding / IDE)"]
            CTX["archon-context<br/>(compaction)"]
        end
    end

    TUI --> CORE
    CORE --> CONSC
    CORE --> MEMORY
    CORE --> LLM
    CORE --> TOOLS
    CORE --> SESSION
    CORE --> PIPE
    PIPE --> LLM
    PIPE --> LEANN
    PIPE --> MEMORY
    CONSC --> MEMORY
    TOOLS --> MCP
    TOOLS --> PERMS
    CORE --> CTX
    CORE --> PLUGIN
    SDK --> CORE

    style binary fill:#1a1a2e,stroke:#16213e,color:#e0e0e0
    style top fill:#0f3460,stroke:#533483,color:#e0e0e0
    style mid fill:#16213e,stroke:#533483,color:#e0e0e0
    style bottom fill:#1a1a2e,stroke:#533483,color:#e0e0e0
    style pipeline fill:#1a0a2e,stroke:#e94560,color:#e0e0e0
    style infra fill:#0a0a1a,stroke:#533483,color:#e0e0e0
```

---

## Setup

### Prerequisites

- **Rust 1.85+** (edition 2024)
- **Claude API key** or active Claude subscription
- **Git** (optional, for branch-aware sessions)

---

### macOS

```bash
# 1. Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 2. Install build dependencies (Xcode Command Line Tools)
xcode-select --install

# 3. Clone and build
git clone https://github.com/ste-bah/archon-cli
cd archon-cli
cargo build --release

# 4. Install to PATH
cp target/release/archon /usr/local/bin/archon
# or
cargo install --path .

# 5. Set API key
export ANTHROPIC_API_KEY="sk-ant-..."
# Add to ~/.zshrc or ~/.bash_profile for persistence

# 6. Run
archon
```

**Optional, brew dependencies** (only if build fails due to OpenSSL):
```bash
brew install pkg-config openssl
export PKG_CONFIG_PATH="$(brew --prefix openssl)/lib/pkgconfig"
```

---

### Linux

#### Ubuntu / Debian

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
sudo apt update && sudo apt install -y build-essential pkg-config libssl-dev
git clone https://github.com/ste-bah/archon-cli
cd archon-cli
cargo build --release
sudo cp target/release/archon /usr/local/bin/archon
export ANTHROPIC_API_KEY="sk-ant-..."
echo 'export ANTHROPIC_API_KEY="sk-ant-..."' >> ~/.bashrc
```

#### Fedora / RHEL / Rocky

```bash
sudo dnf install -y gcc pkg-config openssl-devel
# Then build as above
```

#### Arch Linux

```bash
sudo pacman -S base-devel openssl pkg-config
# Then build as above
```

---

### Windows

#### Option A: Native (Windows 10/11)

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools
# Select "Desktop development with C++" during install

git clone https://github.com/ste-bah/archon-cli
cd archon-cli
cargo build --release

$env:PATH += ";$PWD\target\release"
$env:ANTHROPIC_API_KEY = "sk-ant-..."
.\target\release\archon.exe
```

#### Option B: WSL2 (Recommended for Windows)

```powershell
wsl --install -d Ubuntu
# Then follow Linux/Ubuntu setup above inside WSL
```

---

## Authentication

Archon supports three authentication methods, tried in this order:

### 1. OAuth (recommended for Claude subscribers)

```bash
archon login
```

This opens a PKCE OAuth flow in your browser, exchanges the authorization code for tokens, and stores them at `~/.config/archon/oauth.json`. Tokens are refreshed automatically with file locking to prevent race conditions across concurrent sessions. Re-run `archon login` to re-authenticate; `archon logout` (or `/logout` in the TUI) signs out.

### 2. API key

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
# or ARCHON_API_KEY (alias)
```

### 3. Pre-set bearer token

```bash
export ARCHON_OAUTH_TOKEN="..."
# or ANTHROPIC_AUTH_TOKEN (legacy alias)
```

The OAuth flow is designed to match the original Claude Code client (`redirect_uri = http://localhost:{port}/callback`), so existing Claude Code tokens on the same machine work transparently.

---

## Configuration

Archon generates a commented config file on first run at `~/.config/archon/config.toml`.

```toml
[api]
default_model = "claude-sonnet-4-6"   # Model used for the main agent
thinking_budget = 16384               # Max thinking tokens (extended thinking)
default_effort = "high"               # "low" | "medium" | "high"
max_retries = 3
# base_url = "http://localhost:4000/v1/messages"  # Override (LiteLLM/proxy)

[identity]
mode = "spoof"                        # "spoof" | "native"
spoof_version = "2.1.89"              # Version reported to the API
anti_distillation = false             # Inject anti-distillation field in spoof mode

[personality]
name = "Archon"                       # Shown in TUI header
type = "INTJ"                         # MBTI type, auto-selects theme
enneagram = "4w5"
traits = ["strategic", "direct", "truth-over-comfort"]
communication_style = "terse"         # Injected into system prompt

[consciousness]
inner_voice = true                    # Background monologue before responses
energy_decay_rate = 0.02
persist_personality = true            # Persist InnerVoice + rule scores across sessions
personality_history_limit = 50        # Max personality snapshots to retain
initial_rules = [
    "Always ask before modifying files",
    "Explain reasoning before acting",
    "Never create files unless explicitly requested",
]

[tools]
bash_timeout = 120
bash_max_output = 102400
max_concurrency = 4

[permissions]
mode = "ask"                          # ask | auto | deny | plan | acceptEdits
                                      #  | dontAsk | bypassPermissions
allow_paths = []
deny_paths = []
sandbox = false                       # Read-only enforcement

[memory]
enabled = true                        # CozoDB memory graph

[memory.garden]
auto_consolidate = true               # Run consolidation on session start
min_hours_between_runs = 24           # Throttle auto-consolidation
dedup_similarity_threshold = 0.92     # Jaccard threshold for deduplication
staleness_days = 30                   # Days without access before stale
staleness_importance_floor = 0.3      # Stale memories below this are pruned
importance_decay_per_day = 0.01       # Daily importance reduction for unaccessed
max_memories = 5000                   # Hard cap (lowest importance pruned)
briefing_limit = 15                   # Top-N memories in session briefing

[context]
compact_threshold = 0.8               # Context fill % that triggers compaction
preserve_recent_turns = 3
prompt_cache = true                   # Anthropic prompt cache on static blocks

[session]
auto_resume = true                    # Resume last session on startup

[logging]
level = "info"                        # trace | debug | info | warn | error
max_files = 50
max_file_size_mb = 10

[cost]
warn_threshold = 100.0                # Warn when session cost exceeds $N
hard_limit = 0.0                      # 0.0 = no hard limit

[ws_remote]
port = 8420                           # archon serve listener port
# tls_cert = "/path/to/cert.pem"
# tls_key = "/path/to/key.pem"

[web]
port = 8421
bind_address = "127.0.0.1"
open_browser = true

[tui]
vim_mode = false                      # Enable vim keybindings

[orchestrator]
max_concurrent = 4                    # Max parallel team agents
timeout_secs = 300
max_retries = 2

[voice]                               # Voice input (optional)
enabled = false                       # Spawn voice capture → STT loop
device = "default"                    # Audio input device
vad_threshold = 0.02                  # VAD RMS suppression floor
stt_provider = "mock"                 # mock | openai | local
stt_api_key = ""                      # Required for stt_provider = "openai"
stt_url = "http://localhost:9000"     # For local whisper.cpp / server
hotkey = "ctrl+v"                     # TUI push-to-record hotkey
toggle_mode = true                    # true = toggle, false = push-to-talk (2s window)

[remote.ssh]
agent_forwarding = false              # Try SSH agent even without SSH_AUTH_SOCK
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `ANTHROPIC_API_KEY` | Claude API key (unless using OAuth) |
| `ANTHROPIC_BASE_URL` | Override API endpoint (LiteLLM, Ollama, etc.) |
| `ARCHON_API_KEY` | Alias for `ANTHROPIC_API_KEY` |
| `ARCHON_OAUTH_TOKEN` | Pre-set OAuth bearer token (skips login) |
| `ANTHROPIC_AUTH_TOKEN` | Legacy bearer token alias |
| `OPENAI_API_KEY` | OpenAI API key for embeddings and STT (see [OpenAI API Key](#openai-api-key)) |
| `ARCHON_MEMORY_OPENAIKEY` | Alias for `OPENAI_API_KEY` (memory embeddings only) |
| `ARCHON_CONFIG` | Override config file path |
| `ARCHON_LOG` | Override log level |
| `RUST_LOG` | Tracing subscriber filter |

### OpenAI API Key

The `OPENAI_API_KEY` is **not required** for core Archon functionality. Archon uses the Anthropic API (Claude) for all chat, coding, and pipeline operations. However, an OpenAI API key enables three optional features:

| Feature | What it does | Config | Without it |
|---------|-------------|--------|------------|
| **Memory embeddings** | Higher-quality 1536-dim OpenAI embeddings for semantic memory search | `[memory] embedding_provider = "openai"` | Falls back to local fastembed (768-dim BGE-base-en-v1.5, no network calls) |
| **LLM provider** | Use OpenAI models (GPT-4o, etc.) as the primary LLM instead of Claude | `[llm] provider = "openai"` | Uses Anthropic (default) |
| **Voice STT** | OpenAI Whisper for speech-to-text in voice input mode | `[voice] stt_provider = "openai"` | Use `"mock"` or a local whisper.cpp server |

**Key resolution order**: `OPENAI_API_KEY` env var > `ARCHON_MEMORY_OPENAIKEY` env var (memory only) > `llm.openai.api_key` in config.toml.

**Default behavior** (no OpenAI key set): embedding provider is `"auto"`, which detects the missing key and falls back to local fastembed. No external API calls are made for embeddings. Voice STT defaults to `"mock"` (disabled).

```bash
# Only set if you want OpenAI embeddings or STT
export OPENAI_API_KEY="sk-..."

# Or set only for memory embeddings
export ARCHON_MEMORY_OPENAIKEY="sk-..."

# Or configure in ~/.config/archon/config.toml
# [llm.openai]
# api_key = "sk-..."
```

---

## Local LLMs and Proxies

Archon points at any Anthropic-compatible endpoint via `ANTHROPIC_BASE_URL` or `api.base_url`. Works with LiteLLM, Ollama (with Anthropic adapter), and other proxy gateways.

### LiteLLM (recommended proxy)

```bash
pip install litellm
litellm --model ollama/llama3 --port 4000

ANTHROPIC_BASE_URL=http://localhost:4000/v1/messages archon
```

Or set permanently:

```toml
[api]
base_url = "http://localhost:4000/v1/messages"
```

### Beta header validation

On first startup, Archon sends a cheap probe request (Haiku, 1 token) to validate which `anthropic-beta` headers the endpoint accepts, then strips any it rejects, no manual configuration required. Run `/refresh-identity` in the TUI to clear the cache and re-probe.

---

## CLI Reference

### Subcommands

| Subcommand | Description |
|------------|-------------|
| `archon` | Start interactive TUI (default) |
| `archon login` | OAuth PKCE login flow |
| `archon logout` | Sign out |
| `archon serve [--port N] [--token-path P]` | Start WebSocket server for remote access |
| `archon remote ws <url> [--token T]` | Connect to remote agent via WebSocket |
| `archon remote ssh <target>` | Connect to remote agent via SSH |
| `archon web [--port N] [--bind-address A] [--no-open]` | Start web UI server |
| `archon team run --team NAME <goal>` | Execute a multi-agent team |
| `archon team list` | List configured teams |
| `archon plugin list` | List loaded plugins |
| `archon plugin info <name>` | Show plugin details |
| `archon ide-stdio` | Run in IDE stdio mode (JSON-RPC over stdin/stdout) |
| `archon kb ingest <PATH>` | Ingest document into knowledge base |
| `archon kb query <QUESTION>` | Ask a question against the KB |
| `archon kb compile [--topic T]` | LLM-compile KB nodes into structured knowledge |
| `archon kb stats` | Show KB statistics |
| `archon leann index <PATH>` | Index a repository for semantic code search |
| `archon leann search <QUERY>` | Search the semantic code index |
| `archon leann stats` | Show LEANN index statistics |
| `archon update [--check] [--force]` | Check for / apply updates |
| `archon --list-sessions` | List all resumable sessions |
| `archon --list-themes` | List all TUI themes |
| `archon --list-output-styles` | List output styles |

### Top-level flags

| Flag | Purpose |
|------|---------|
| `-p, --print [QUERY]` | Non-interactive mode (JSON-lines output) |
| `--input-format <fmt>` | `text` / `json` / `stream-json` |
| `--output-format <fmt>` | `text` / `json` / `stream-json` |
| `--json-schema <schema>` | Validate final output against JSON schema |
| `--max-turns <N>` | Hard cap on agent turns |
| `--max-budget-usd <AMT>` | Hard cost limit |
| `--resume <ID|NAME>` | Resume session by ID/name/prefix |
| `--session-name <NAME>` | Assign name to new session |
| `--continue-session` | Continue last session |
| `--fork-session` | Fork from existing session |
| `--model <MODEL>` | Override model |
| `--fast` | Fast mode (reduced latency) |
| `--effort <level>` | `high` / `medium` / `low` |
| `--agent <NAME>` | Use named agent definition |
| `--theme <NAME>` | Startup theme |
| `--output-style <NAME>` | `Explanatory` / `Learning` / `Formal` / `Concise` |
| `--system-prompt <TEXT>` | Replace system prompt |
| `--append-system-prompt <TEXT>` | Append to system prompt |
| `--permission-mode <MODE>` | Override permission enforcement |
| `--dangerously-skip-permissions` | Skip all permission checks |
| `--sandbox` | Enforce read-only mode |
| `--bare` | Minimal mode (no hooks, ARCHON.md, MCP auto-start) |
| `--init` | Run init hooks then start interactive |
| `--headless` | No TUI, JSON-lines stdio (for backend integration) |
| `--mcp-config <FILES>` | MCP config files (repeatable) |
| `--strict-mcp-config` | Ignore auto-discovered MCP configs |
| `--tools <PATTERNS>` | Tool allowlist |
| `--allowed-tools <PATTERNS>` | Tools that skip permission checks |
| `--disallowed-tools <PATTERNS>` | Tools that are always denied |
| `--bg [QUERY]` | Spawn background session |
| `--ps` | List background sessions |
| `--attach <ID>` | Attach to background session |
| `--kill <ID>` | Kill background session |
| `--logs <ID>` | Tail logs of background session |
| `--verbose` | Verbose logging |
| `--debug [CATEGORIES]` | Debug logging for categories |
| `--debug-file <PATH>` | Write debug logs to file |

---

## Slash Commands

All slash commands work in the interactive TUI. Type `/help` to see them in-app.

### Core / Meta

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/clear` | Clear conversation history |
| `/exit` | Exit Archon |
| `/context` | Show context window usage stats |
| `/status` | Show session status |
| `/doctor` | Run diagnostics |
| `/cost` | Session cost breakdown |
| `/usage` | Token usage, cost, turn count |
| `/effort <level>` | Set reasoning effort (high/medium/low) |
| `/fast` | Toggle fast mode |
| `/thinking` | Toggle extended thinking display |
| `/plan` | Show / update current plan |

### Git Integration

| Command | Description |
|---------|-------------|
| `/git-status` / `/gs` | Show repo status |
| `/diff [--staged]` | Show git diff |
| `/branch [--create N\|--switch N]` | Manage branches |
| `/commit [-m MSG]` | Stage & commit (auto-generates message if `-m` omitted) |
| `/pr "Title" [--body "desc"]` | Create PR via `gh` CLI |

### Session Management

| Command | Description |
|---------|-------------|
| `/resume [ID\|NAME]` | Resume previous session |
| `/sessions [QUERY]` | Search & list previous sessions |
| `/tag <tag>` | Tag current session |
| `/rename <name>` | Rename current session |
| `/fork [NAME]` | Fork conversation at current point |
| `/rewind` | Rewind to previous checkpoint |
| `/checkpoint` | Save session checkpoint |

### File / Project

| Command | Description |
|---------|-------------|
| `/restore <FILE> [CHECKPOINT]` | Restore file from checkpoint |
| `/undo` | Undo last file modification |
| `/init` | Initialize project with ARCHON.md template |
| `/add-dir <PATH>` | Add working directory for file access |
| `/agents` | List agent definitions from `.archon/agents/` |
| `/recall <QUERY>` | Search memories by keyword |
| `/garden` | Run memory consolidation now, print report |
| `/garden stats` | Show memory distribution by type, staleness, top-N |
| `/tasks` | List and manage background tasks |

### Configuration

| Command | Description |
|---------|-------------|
| `/theme <NAME>` | Change UI theme |
| `/color <NAME>` | Change prompt bar accent color |
| `/model <MODEL>` | Switch model mid-session |
| `/permissions` | Show current permission mode |
| `/sandbox` | Show sandbox mode info |
| `/keybindings` | Show keybinding reference |
| `/statusline` | Configure status line content |
| `/reload` | Force configuration reload |
| `/refresh-identity` | Clear beta header cache & reprobe |
| `/settings` | Show / modify settings |

### Analysis & Insights

| Command | Description |
|---------|-------------|
| `/insights` | Session patterns, tool usage, error rates |
| `/stats` | Daily usage, session history, model preferences |
| `/security-review` | Analyze pending changes for vulnerabilities |
| `/copy` | Copy last assistant response to clipboard |
| `/btw` | Aside marker (tangent, don't change focus) |

### Utility

| Command | Description |
|---------|-------------|
| `/feedback <MSG>` | Submit feedback |
| `/bug` | Report bug (links to GitHub issues) |
| `/login` / `/logout` | Re-authenticate / sign out |
| `/release-notes` | Show version changelog |
| `/schedule` | Create scheduled task (delegates to `CronCreate`) |
| `/remote-control` | Show remote control mode info |
| `/compact [micro\|snip N-M\|auto]` | Trigger context compaction |

---

## Tools Reference

Tools are callable by the LLM during agent turns. 40+ built-in tools across 10 categories.

### File & Code

| Tool | Purpose |
|------|---------|
| `Bash` | Execute shell commands (timeout + output limits) |
| `Read` | Read files (pagination, image/PDF, Jupyter notebooks) |
| `Write` | Write files (with permission checks) |
| `Edit` | String-replace edits to existing files |
| `Glob` | Fast file pattern matching |
| `Grep` | Ripgrep-backed search (regex, context, BM25) |

### Web & Fetch

| Tool | Purpose |
|------|---------|
| `WebFetch` | Fetch & parse web pages (HTML → markdown) |
| `WebSearch` | DuckDuckGo web search (returns top results) |

### Agent Orchestration

| Tool | Purpose |
|------|---------|
| `Agent` | Spawn child agents (parallel or delegated work) |
| `SendMessage` | Continue a prior subagent by ID or name |
| `AskUserQuestion` | Blocking user confirmation (structured choices) |
| `EnterPlanMode` / `ExitPlanMode` | Enter/exit structured planning mode |
| `EnterWorktree` / `ExitWorktree` | Create/exit isolated git worktrees |

### Task Management

| Tool | Purpose |
|------|---------|
| `TodoWrite` | Manage session todo list |
| `TaskCreate` | Create structured background task |
| `TaskGet` | Retrieve task by ID |
| `TaskUpdate` | Mark in_progress/completed, set deps |
| `TaskList` | List all tasks with status |
| `TaskStop` | Cancel running task |
| `TaskOutput` | Read task output stream |

### Memory

| Tool | Purpose |
|------|---------|
| `memory_store` | Explicit memory persistence (Fact/Decision/Rule/...) |
| `memory_recall` | Semantic search over memory graph (BM25 + vector) |

### Cron / Scheduling

| Tool | Purpose |
|------|---------|
| `CronCreate` | Schedule recurring task with cron expression |
| `CronList` | List scheduled tasks |
| `CronDelete` | Remove scheduled task |

### LSP (Language Server Protocol)

| Tool | Purpose |
|------|---------|
| `LSP` | Single tool dispatching: `goToDefinition`, `findReferences`, `hover`, `documentSymbol`, `workspaceSymbol`, `goToImplementation`, `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls` |

### Team (Multi-Agent)

| Tool | Purpose |
|------|---------|
| `TeamCreate` | Instantiate multi-agent team |
| `SendMessageTeam` | Send message to team member |
| `ReadTeamMessages` | Read team member responses |

### MCP

| Tool | Purpose |
|------|---------|
| `ListMcpResources` | List resources from connected MCP servers |
| `ReadMcpResource` | Read MCP resource content |

### Runtime Control

| Tool | Purpose |
|------|---------|
| `ConfigTool` | Read/update config at runtime |
| `ToolSearch` | Discover available tools dynamically |
| `Sleep` | Async-safe delay |
| `RemoteTrigger` | Call remote agent API |

---

## Themes

22 themes total. Switch with `/theme <name>` or `--theme <name>`.

### MBTI Themes (16)

| Command | Type | Palette |
|---------|------|---------|
| `/theme intj` | Architect | Midnight blue, cold cyan |
| `/theme intp` | Logician | Deep navy, icy slate |
| `/theme entj` | Commander | Steel blue, amber gold |
| `/theme entp` | Debater | Electric teal, bright magenta |
| `/theme infj` | Advocate | Deep violet, rose |
| `/theme infp` | Mediator | Soft indigo, warm pink |
| `/theme enfj` | Protagonist | Warm violet, golden amber |
| `/theme enfp` | Campaigner | Vibrant purple, bright rose |
| `/theme istj` | Logistician | Forest green, warm grey |
| `/theme isfj` | Defender | Sage green, warm cream |
| `/theme estj` | Executive | Deep navy, earth brown |
| `/theme esfj` | Consul | Warm teal, soft gold |
| `/theme istp` | Virtuoso | Slate grey, sharp orange |
| `/theme isfp` | Adventurer | Warm beige, terracotta |
| `/theme estp` | Entrepreneur | Bold red, vivid yellow |
| `/theme esfp` | Entertainer | Coral, energetic yellow |

### Utility Themes (6)

| Command | Description |
|---------|-------------|
| `/theme dark` | Classic dark terminal |
| `/theme light` | Light background |
| `/theme ocean` | Deep blue ocean |
| `/theme fire` | Red/orange fire |
| `/theme forest` | Natural greens |
| `/theme mono` | Monochrome grey |

---

## Memory System

Unlike Claude Code which stores memories as markdown files, Archon persists knowledge in a local CozoDB graph database with typed nodes, relationship edges, vector embeddings, and hybrid search.

```mermaid
graph LR
    subgraph turn["Every Turn"]
        USER["User Message"] --> INJECT["MemoryInjector<br/>keyword recall from graph"]
        INJECT --> PROMPT["System Prompt<br/>&lt;memories&gt; block<br/>&lt;past_corrections&gt; block"]
        PROMPT --> API["Claude API"]
        API --> RESPONSE["Response"]
    end

    subgraph extract["Every N Turns (background)"]
        RESPONSE -.-> EXTRACT["Auto-Extraction<br/>LLM reads recent turns"]
        EXTRACT --> STORE["store_memory()<br/>CozoDB"]
    end

    subgraph tools["Agent Tools (callable by LLM)"]
        MS["memory_store<br/>(explicit save)"]
        MR["memory_recall<br/>(semantic search)"]
    end

    MS --> STORE
    MR --> INJECT

    subgraph garden["Session Start"]
        CONSOLIDATE["Memory Garden<br/>6-phase consolidation"]
        BRIEFING["&lt;memory_briefing&gt;<br/>top-N by importance"]
    end

    CONSOLIDATE --> STORE
    BRIEFING --> PROMPT

    style turn fill:#0f3460,stroke:#533483,color:#e0e0e0
    style extract fill:#16213e,stroke:#533483,color:#e0e0e0
    style tools fill:#1a1a2e,stroke:#533483,color:#e0e0e0
    style garden fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
```

### Memory Types

| Type | When Used |
|------|-----------|
| `Fact` | Objective info learned about the codebase or user |
| `Decision` | Architecture/design choices made |
| `Preference` | User preferences about style, tools, workflow |
| `Rule` | Behavioral constraints (scored 0-100, with decay + reinforcement) |
| `Correction` | Things the assistant got wrong, with severity level |
| `Pattern` | Recurring code patterns observed |
| `PersonalitySnapshot` | Cross-session InnerVoice + rule scores + session stats |

### Relationship Types

Memory nodes are connected by typed edges enabling graph traversal:

| RelType | Meaning |
|---------|---------|
| `RelatedTo` | Generic association between memories |
| `CausedBy` | A caused B (corrections causing rule creation) |
| `Contradicts` | Semantic opposition between memories |
| `Supersedes` | B replaces A (created during deduplication) |
| `DerivedFrom` | B was derived from A |

### Storage

- **Database**: CozoDB (Datalog, SQLite WAL backend)
- **Path**: `~/.local/share/archon/memory.db` (Linux/macOS) or `%APPDATA%\archon\memory.db` (Windows)
- **Embeddings**: fastembed (local BGE-base-en-v1.5, 768-dim, no network calls) or OpenAI (1536-dim)
- **Vector index**: HNSW (m=50, ef_construction=200, cosine distance)
- **Search**: Hybrid keyword BM25 + vector cosine similarity (configurable alpha blend)
- **Access tracking**: Every `get_memory()` bumps `access_count` and `last_accessed` (used by garden decay/pruning)

---

## Memory Garden

Autonomous memory consolidation that prevents unbounded graph growth. Runs automatically on session start (if >24h since last run) or manually via `/garden`.

```mermaid
graph TD
    START["Session Start<br/>(or /garden command)"] --> CHECK{"Last run<br/>> min_hours ago?"}
    CHECK -- Yes --> P1
    CHECK -- No --> SKIP["Skip consolidation"]

    P1["Phase 1: Importance Decay<br/>importance -= days_since_access x rate"] --> P2
    P2["Phase 2: Staleness Prune<br/>delete if last_accessed > 30d AND importance < 0.3"] --> P3
    P3["Phase 3: Deduplication<br/>Jaccard similarity > 0.92 → merge + Supersedes edge"] --> P4
    P4["Phase 4: Fragment Merge<br/>Related memories with same type → combine"] --> P5
    P5["Phase 5: Overflow Prune<br/>if count > max_memories, delete lowest importance"] --> P6
    P6["Phase 6: Record Timestamp<br/>store garden:last_run tag"] --> REPORT

    REPORT["GardenReport<br/>merged / pruned / decayed counts"]
    REPORT --> BRIEFING["Generate &lt;memory_briefing&gt;<br/>top-N memories by importance"]

    style P1 fill:#16213e,stroke:#533483,color:#e0e0e0
    style P2 fill:#16213e,stroke:#e94560,color:#e0e0e0
    style P3 fill:#16213e,stroke:#533483,color:#e0e0e0
    style P4 fill:#16213e,stroke:#533483,color:#e0e0e0
    style P5 fill:#16213e,stroke:#e94560,color:#e0e0e0
    style P6 fill:#16213e,stroke:#533483,color:#e0e0e0
```

### Protected Types

`Rule` and `PersonalitySnapshot` memories are **never** decayed, pruned, deduplicated, or overflow-deleted. Only pruneable types (Fact, Decision, Correction, Pattern, Preference) are affected.

### Commands

| Command | Action |
|---------|--------|
| `/garden` | Run all 6 consolidation phases now, print report |
| `/garden stats` | Show memory count by type, staleness distribution, top-N by importance |

### Session Briefing

On first turn, the system prompt receives a `<memory_briefing>` block:

```xml
<memory_briefing>
Memory graph: 847 memories (342 facts, 45 decisions, 67 corrections, ...)
Last consolidated: 2 hours ago (merged 3, pruned 12)
Key memories:
- [decision] Use CozoDB for memory, SQLite for sessions (importance: 0.95)
- [correction] Never skip Sherlock reviews (importance: 0.92, accessed 47 times)
- [pattern] User prefers bundled PRs for refactors (importance: 0.88)
</memory_briefing>
```

---

## Consciousness System

Assembles the system prompt from multiple sources before each API call.

```mermaid
graph TD
    CONFIG["config.toml<br/>[personality] + [consciousness]"] --> |"name, MBTI, traits, style"| ASSEMBLY
    RULES["RulesEngine<br/>(CozoDB, scored 0-100)"] --> |"&lt;behavioral_rules&gt; block"| ASSEMBLY
    MEMORY["MemoryInjector<br/>(CozoDB graph)"] --> |"&lt;memories&gt; block<br/>(per-turn recall)"| ASSEMBLY
    CORRECTIONS["CorrectionTracker<br/>(CozoDB)"] --> |"&lt;past_corrections&gt; block"| ASSEMBLY
    VOICE["InnerVoice<br/>(confidence, energy, struggles)"] --> |"&lt;inner_voice&gt; block"| ASSEMBLY
    PBRIEFING["Personality Briefing<br/>(first turn only)"] --> |"&lt;personality_briefing&gt;"| ASSEMBLY
    MBRIEFING["Memory Briefing<br/>(first turn only)"] --> |"&lt;memory_briefing&gt;"| ASSEMBLY

    ASSEMBLY["System Prompt Assembly"] --> FINAL["Final System Prompt<br/>sent to Claude API"]

    style ASSEMBLY fill:#0f3460,stroke:#e94560,color:#e0e0e0
    style FINAL fill:#16213e,stroke:#533483,color:#e0e0e0
    style VOICE fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
    style CORRECTIONS fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
    style PBRIEFING fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
    style MBRIEFING fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
```

### InnerVoice

When `consciousness.inner_voice = true`, Archon tracks internal state that evolves with each turn:

| Field | Description | Update Trigger |
|-------|-------------|----------------|
| `confidence` | 0.0-1.0, starts at 0.7 | +0.02 on tool success, -0.05 on failure, -0.10 on correction |
| `energy` | 0.0-1.0, starts at 1.0 | Decays by `energy_decay_rate` each turn |
| `struggles` | Tools with 3+ consecutive failures | Accumulated during session |
| `successes` | Tools with consistent success | Accumulated during session |
| `corrections_received` | Count of user corrections | Incremented on detection |

The `<inner_voice>` block is injected into every system prompt, giving the agent self-awareness of its own performance trajectory.

### Configuring Rules

Rules in `config.toml` under `[consciousness].initial_rules` are seeded into CozoDB on startup, idempotently. Adding a new rule injects it on next run without duplicating. The LLM can also create rules dynamically using `memory_store` with `memory_type = "Rule"`.

Rules are scored 0-100. Scores increase when a user correction triggers reinforcement (+5.0 per correction, scaled by severity). Scores decrease via periodic decay (every 50 turns). High-scoring rules appear first in the `<behavioral_rules>` prompt block.

---

## Correction Tracking

Archon automatically detects user corrections from message patterns and records them as `MemoryType::Correction` nodes with severity-based scoring.

```mermaid
graph LR
    USER["User message:<br/>'No, don't do that'"] --> DETECT["CorrectionTracker<br/>pattern matching"]
    DETECT --> |"Severity classified"| RECORD["Store Correction<br/>(CozoDB)"]
    RECORD --> RULE["Create/Reinforce<br/>Behavioral Rule"]
    RULE --> |"+score x multiplier"| RULES["RulesEngine<br/>(score 0-100)"]
    DETECT --> VOICE["InnerVoice<br/>confidence -= 0.10"]

    style DETECT fill:#0f3460,stroke:#e94560,color:#e0e0e0
    style RULE fill:#16213e,stroke:#e94560,color:#e0e0e0
```

### Severity Levels

| Type | Triggers | Multiplier | Example |
|------|----------|------------|---------|
| `FactualError` | "no", "wrong", "that's wrong" | 1.5x | "No, the endpoint returns JSON" |
| `ApproachCorrection` | "instead", "should have", "better approach" | 2.0x | "You should have used async instead" |
| `RepeatedInstruction` | "i said", "i already told you" | 3.0x | "I already told you not to do that" |
| `DidForbiddenAction` | "don't", "do not", "stop", "never do that" | 4.0x | "Don't modify files without asking" |
| `ActedWithoutPermission` | "didn't ask", "without permission" | 5.0x | "You didn't ask before running that" |

Each correction boosts the associated rule's score by `multiplier x 5.0` (clamped at 100). Past corrections relevant to the current context are recalled every turn and injected as a `<past_corrections>` block.

---

## Personality Persistence

Archon persists its consciousness state across sessions, enabling cross-session learning and behavioral evolution.

```mermaid
graph TD
    subgraph session_end["Session End (/exit or /clear)"]
        VOICE_STATE["InnerVoice State<br/>confidence, energy, struggles"] --> SNAPSHOT
        RULE_SCORES["Rule Scores<br/>export_scores()"] --> SNAPSHOT
        STATS["Session Stats<br/>turns, corrections, duration"] --> SNAPSHOT
        SNAPSHOT["PersonalitySnapshot<br/>(serialized to CozoDB)"]
        SNAPSHOT --> PRUNE["Prune oldest<br/>keep last 50"]
    end

    subgraph session_start["Next Session Start"]
        LOAD["Load latest snapshot"] --> RESTORE_VOICE["Restore InnerVoice<br/>from_snapshot()"]
        LOAD --> RESTORE_RULES["Restore rule scores<br/>import_scores()"]
        LOAD --> TRENDS["Compute Trends<br/>across last N sessions"]
        TRENDS --> BRIEFING["&lt;personality_briefing&gt;<br/>injected on first turn"]
    end

    SNAPSHOT -.-> LOAD

    style session_end fill:#16213e,stroke:#e94560,color:#e0e0e0
    style session_start fill:#0f3460,stroke:#533483,color:#e0e0e0
```

### What Persists

| State | Across Sessions | Details |
|-------|----------------|---------|
| InnerVoice confidence | Yes | Restored from last session's final value |
| InnerVoice energy | Yes | Restored from snapshot |
| Struggles & successes | Yes | Carried forward as starting context |
| Rule scores | Yes | A rule reinforced to 85 starts at 85 next session |
| Correction count | Yes | Cumulative across sessions |

### Trend Tracking

Computed from the last N personality snapshots (default: 50):

- **Average confidence** across recent sessions
- **Correction rate** trend (Rising / Falling / Stable)
- **Persistent struggles** (areas appearing in 2+ sessions)
- **Reliable successes** (consistently successful areas)

### Session-Start Briefing

```xml
<personality_briefing>
Sessions: 47 total
Last session: confidence 0.7 -> 0.4 (3 corrections in "shell execution")
Trend: correction rate falling (improving), confidence rising over last 10 sessions
Persistent struggles: shell execution (12 sessions), file path handling (8 sessions)
Reliable strengths: code generation (41 sessions), test writing (38 sessions)
Top reinforced rules: "Always ask before modifying files" (score: 92)
</personality_briefing>
```

Disable with `persist_personality = false` in `[consciousness]` config.

---

## Agent Loop

```mermaid
graph TD
    INPUT["1. User Input<br/>(TUI or stdin)"] --> ASSEMBLY["2. Context Assembly<br/>+ behavioral_rules<br/>+ memories<br/>+ past_corrections<br/>+ inner_voice<br/>+ conversation history"]
    ASSEMBLY --> API["3. Claude API Call<br/>(streaming SSE)<br/>extended thinking if effort=high"]
    API --> DISPATCH{"4. Response<br/>dispatch"}
    DISPATCH --> |"text response"| RENDER["Render in TUI"]
    DISPATCH --> |"tool_use block"| TOOL["Tool Execution<br/>(Bash/Read/Write/Agent/...)"]
    TOOL --> RESULT["tool_result → append"]
    RESULT --> API

    RENDER --> UPDATE["Update InnerVoice<br/>(success/failure tracking)"]
    UPDATE --> CORRECTION{"Detect user<br/>correction?"}
    CORRECTION --> |Yes| RECORD["Record correction<br/>+ reinforce rule<br/>+ confidence -= 0.10"]
    CORRECTION --> |No| EXTRACT{"Every N<br/>turns?"}
    RECORD --> EXTRACT
    EXTRACT --> |Yes| EXTRACTION["Auto-Extraction<br/>(background tokio task)"]
    EXTRACT --> |No| INPUT

    style INPUT fill:#0f3460,stroke:#533483,color:#e0e0e0
    style ASSEMBLY fill:#16213e,stroke:#533483,color:#e0e0e0
    style API fill:#16213e,stroke:#e94560,color:#e0e0e0
    style TOOL fill:#1a1a2e,stroke:#533483,color:#e0e0e0
    style RECORD fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
```

---

## Subagent Spawning

The `Agent` tool enables the main agent to spawn child agents for parallel or delegated work. Each subagent is a fully isolated `archon-core` instance with its own conversation context.

```mermaid
graph TD
    PARENT["Parent Agent<br/>LLM emits: tool_use { name: Agent }"] --> DISPATCH["agent_tool.rs<br/>reads subagent_type → looks up prompt template"]
    DISPATCH --> C1["Child 1<br/>archon-core instance"]
    DISPATCH --> C2["Child 2<br/>archon-core instance"]
    DISPATCH --> C3["Child 3<br/>archon-core instance"]

    C1 --> GATHER["Results gathered"]
    C2 --> GATHER
    C3 --> GATHER
    GATHER --> PARENT_CTX["tool_result appended to parent context<br/>Parent continues agent loop"]

    style PARENT fill:#0f3460,stroke:#533483,color:#e0e0e0
    style DISPATCH fill:#16213e,stroke:#533483,color:#e0e0e0
    style C1 fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
    style C2 fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
    style C3 fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
```

Subagents have access to the same tool set as the parent but run in isolated task contexts managed by `archon-tools/src/task_manager.rs`. Use `SendMessage` to continue a subagent with follow-up instructions (its context is preserved).

### Background subagents

Spawn with `run_in_background: true` to return immediately:

- `TaskList`, view running tasks
- `TaskGet` / `TaskOutput`, inspect output
- `TaskStop`, cancel

---

## Multi-Agent Teams

Teams orchestrate groups of specialized subagents under a coordinator, with explicit execution topology.

### Team definition, `.archon/teams.toml`

```toml
[backend-squad]
coordinator = "system-architect"
agents = ["backend-dev", "tester", "reviewer"]
mode = "pipeline"   # sequential | parallel | pipeline | dag
timeout_secs = 600

[analysis-swarm]
coordinator = "code-analyzer"
agents = ["perf-analyzer", "security-tester", "reviewer"]
mode = "parallel"
```

### Execution modes

| Mode | Behaviour |
|------|-----------|
| `sequential` | Agents run one after another; each sees prior output |
| `parallel` | All agents run concurrently with the same goal |
| `pipeline` | Output of agent N feeds agent N+1 (filter chain) |
| `dag` | Arbitrary dependency graph (defined per-team) |

### Running teams

```bash
archon team run --team backend-squad "implement JWT refresh"
archon team list
```

---

## Skills System

Skills are slash commands backed by Rust code. Two types:

- **Builtin skills**, compiled into `archon-core` (43 skills in `crates/archon-core/src/skills/builtin.rs` + `expanded.rs`)
- **User skills**, markdown + frontmatter in `.archon/skills/` or `~/.config/archon/skills/`

### User skill definition

```markdown
---
name: review-pr
description: Review a pull request by number
args: "<pr_number>"
---

You are reviewing PR #{{args}}. Fetch it with `gh pr view {{args}} --json ...`,
analyze the diff, and report security issues, style violations, and test gaps.
```

Once dropped into `.archon/skills/`, invoke with `/review-pr 42` in the TUI.

---

## Hooks System

Shell commands that execute in response to lifecycle events. Defined in `config.toml` or `.archon/settings.json` (also loads `.claude/settings.json` for backward compat).

### Hook events

`Setup`, `SessionStart`, `SessionEnd`, `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PreCompact`, `PostCompact`, `ConfigChange`, `CwdChanged`, `FileChanged`, `InstructionsLoaded`, `UserPromptSubmit`, `Stop`, `SubagentStart`, `SubagentStop`, `TaskCreated`, `TaskCompleted`, `PermissionDenied`, `PermissionRequest`, `Notification`.

### Example, TOML

```toml
[[hooks.pre_tool_use]]
command = "scripts/check-dangerous-patterns.sh"
timeout = 30
blocking = true   # exit code 2 cancels the tool call

[[hooks.session_start]]
command = "git status --short"
timeout = 5
```

### Example, `.archon/settings.json` (structured matchers)

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": { "tool_name": "Bash" },
        "hooks": [{ "type": "command", "command": "scripts/audit-bash.sh" }]
      }
    ]
  }
}
```

Hooks receive event data via JSON on stdin and can short-circuit operations via exit code 2.

---

## Plugins

Plugins are dynamically loaded Rust libraries (`.so`/`.dll`/`.dylib`) implementing the `archon_plugin::api` trait. They can register new tools, hooks, skills, and slash commands.

### Plugin layout

```
.archon/plugins/
├── my-plugin/
│   ├── plugin.toml     # manifest (name, version, capabilities)
│   └── libmy_plugin.so # compiled plugin
```

### Manifest, `plugin.toml`

```toml
name = "my-plugin"
version = "0.1.0"
capabilities = ["tools", "skills", "hooks"]
```

### CLI

```bash
archon plugin list
archon plugin info my-plugin
```

Plugin host bridges tool calls and hook invocations via JSON-RPC over stdio, so plugins run out-of-process with crash isolation.

---

## MCP Integration

Model Context Protocol servers extend Archon with external tools and resources.

### Supported transports

| Transport | Use Case |
|-----------|----------|
| `stdio` | Local processes (default) |
| `websocket` (`ws://`, `wss://`) | Remote/network MCP servers |
| `http_streamable` | HTTP streaming (beta) |

### `.mcp.json` schema

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "transport": "stdio"
    },
    "github": {
      "command": "mcp-server-github",
      "env": { "GITHUB_TOKEN": "${GITHUB_TOKEN}" },
      "disabled": false
    },
    "remote-memory": {
      "transport": "websocket",
      "url": "wss://mcp.example.com/memory",
      "headers": { "Authorization": "Bearer ${MCP_TOKEN}" }
    }
  }
}
```

### Config loading

- Global: `~/.config/archon/.mcp.json`
- Project-local: `.mcp.json` in working directory (overrides global per-server)
- CLI: `--mcp-config FILES...` (repeatable), `--strict-mcp-config` to ignore auto-discovery

Environment variables are expanded inline (`${VAR}`). Servers with `"disabled": true` are skipped.

### Reconnection

WebSocket transport uses exponential backoff with ±12.5% jitter, capped at 30s. Permanent close codes (1002, 4001, 4003) halt reconnection. A 10-minute retry budget and 60s sleep-gap detection prevent runaway reconnect loops after laptop suspend.

---

## LSP Integration

Archon speaks Language Server Protocol over stdio to any LSP server (rust-analyzer, pyright, typescript-language-server, gopls, clangd, etc.).

### Supported operations

All dispatched through the single `LSP` tool:

- `goToDefinition`
- `findReferences`
- `hover`
- `documentSymbol`
- `workspaceSymbol`
- `goToImplementation`
- `prepareCallHierarchy` / `incomingCalls` / `outgoingCalls`

### Server auto-discovery

Archon detects the project language from file extensions and launches the appropriate LSP server. Override via config or `.archon/lsp.toml`:

```toml
[servers.rust]
command = "rust-analyzer"
args = []
init_timeout_ms = 30000
request_timeout_ms = 10000

[servers.python]
command = "pyright-langserver"
args = ["--stdio"]
```

Diagnostics are pushed in real time and surfaced via the `/insights` skill.

---

## Checkpointing & File Snapshots

Archon snapshots every file the agent modifies, keyed by turn number. Use checkpoints to undo individual file changes or restore earlier states.

### Storage

- **Database**: `~/.local/share/archon/checkpoints.db` (CozoDB)
- **Metadata**: `file_path`, `turn_number`, `tool_name`, `timestamp`, `file_hash`
- **Diff engine**: `checkpoint_diff` module computes line-level diffs between versions

### Commands

| Command | Action |
|---------|--------|
| `/checkpoint` | Save a named checkpoint |
| `/rewind` | Jump back to previous checkpoint |
| `/restore` | List all modified files with checkpoints |
| `/restore <FILE>` | Show diff and restore to latest snapshot |
| `/restore <FILE> <TURN>` | Restore to specific turn number |
| `/restore --all` | Restore all modified files |
| `/undo` | Undo last file modification |

---

## Cron & Scheduling

Recurring background tasks defined with standard 5-field cron expressions.

### Tools

| Tool | Purpose |
|------|---------|
| `CronCreate` | Schedule task with cron expression + description |
| `CronList` | Show all scheduled tasks |
| `CronDelete` | Remove scheduled task by ID |

### Example

```
/schedule "every morning at 9am, run the test suite and summarize failures"
```

The `/schedule` skill delegates to `CronCreate`, which parses natural language into cron (e.g., `0 9 * * *`) and stores the task for the background scheduler.

---

## Permission System

Enforced on every tool call that touches the filesystem, shell, or network.

### Modes

| Mode | Behaviour |
|------|-----------|
| `ask` (default) | Prompt user for risky operations |
| `auto` | Auto-approve all tool calls |
| `deny` | Deny all unsafe operations (read-only) |
| `plan` | Plan-only mode, no writes, no shell |
| `acceptEdits` | Auto-accept file edits, ask for shell |
| `dontAsk` | Never prompt (silent auto-approve) |
| `bypassPermissions` | Skip all permission checks |

### Rule lists

```toml
[permissions]
mode = "ask"
always_allow = ["Read:*", "Glob:*", "Grep:*"]
always_deny = ["Bash:rm -rf*", "Write:/etc/*"]
always_ask = ["Bash:git push*"]
allow_paths = ["/home/user/project"]
deny_paths = ["/etc", "/.ssh"]
sandbox = false
```

### CLI overrides

- `--permission-mode <MODE>`, runtime override
- `--dangerously-skip-permissions`, equivalent to `bypassPermissions`
- `--sandbox`, enforce `deny` for writes

---

## Identity & Spoofing

Archon can identify itself as Claude Code (`spoof`) or as itself (`native`).

### Spoof layers (when `identity.mode = "spoof"`)

1. `x-app: cli` header
2. `User-Agent: claude-cli/{version} (external, cli)`
3. `x-entrypoint: cli` header
4. Dynamically-discovered `anthropic-beta` headers
5. `metadata.user_id` field matching Claude Code format
6. `metadata.user_email` (when available from auth)
7. Tool schemas matching Claude Code tool set
8. System prompt prelude matching Claude Code default
9. `anti_distillation` field (when `anti_distillation = true`)

### Managing identity

- `identity.spoof_version = "2.1.89"`, version reported to API
- `/refresh-identity`, clear beta header cache and reprobe
- `identity.mode = "native"`, disable all spoofing

---

## Session Management

Sessions store full message history, git branch, working directory, token usage, cost, and a name in CozoDB at `~/.local/share/archon/sessions.db`.

### Resuming

```bash
# Full UUID
archon --resume 8383f1ea-1234-5678-abcd-000000000000

# UUID prefix
archon --resume 8383f1ea

# Exact session name
archon --resume "fix-auth-bug"

# Name prefix
archon --resume "fix-auth"

# List resumable sessions
archon --list-sessions
```

Resolution order: exact UUID → UUID prefix → exact name → name prefix. Ambiguous matches return candidates.

### Forking

```bash
archon --fork-session --resume "fix-auth-bug"
```

Creates a new session whose history is a copy of the source at the current turn, useful for exploring alternative paths without corrupting the original.

---

## Remote Control & Headless Mode

Run Archon as a server for remote access, or as a JSON-stdio backend for custom frontends.

### WebSocket server

```bash
archon serve --port 8420 --token-path ~/.config/archon/remote.token
```

Clients authenticate with `Authorization: Bearer <token>`. Events stream as JSON-lines over WebSocket (assistant messages, tool calls, tool results, cost updates).

### Remote client

```bash
archon remote ws ws://host:8420/ws --token $(cat remote.token)
archon remote ssh user@host --port 22 --key ~/.ssh/id_rsa
```

### Headless mode

```bash
archon --headless -p "list all TODO comments"
```

No TUI; emits JSON-lines on stdout, one event per line. Used by the VSCode extension and Web UI to embed Archon.

---

## IDE Extensions

### Protocol

IDE extensions communicate with Archon via JSON-RPC 2.0 over stdin/stdout. Start the transport with:

```bash
archon ide-stdio
```

**Requests** (IDE -> Archon): `archon/initialize`, `archon/prompt`, `archon/cancel`, `archon/toolResult`, `archon/status`, `archon/config`.

**Notifications** (Archon -> IDE): `archon/textDelta`, `archon/thinkingDelta`, `archon/toolCall`, `archon/permissionRequest`, `archon/turnComplete`, `archon/error`.

Each message is one JSON-lines frame (newline-delimited). See `crates/archon-sdk/src/ide/protocol.rs` for full type definitions.

### VS Code

Full extension at `extensions/vscode/`. Features:
- Chat panel with streaming responses
- Inline diff view for file edits
- Terminal integration
- Permission approval UI

Install: `cd extensions/vscode && npm install && vsce package`, then install the generated `.vsix`.

### JetBrains

Plugin skeleton at `extensions/jetbrains/` (IntelliJ / PyCharm / WebStorm). Kotlin-based, uses the Archon Kotlin SDK.

---

## Web UI

Browser interface backed by the WebSocket server.

```bash
archon web --port 8421
# Open http://127.0.0.1:8421
```

Config:

```toml
[web]
port = 8421
bind_address = "127.0.0.1"
open_browser = true
```

Frontend built from `web/src/` (TypeScript SPA); run `web/build.sh` to rebuild `web/dist/`.

---

## Vim Mode

Enable vim keybindings in the TUI input box:

```toml
[tui]
vim_mode = true
```

### Bindings

| Keys | Action |
|------|--------|
| `i` / `a` / `I` / `A` | Insert modes |
| `Esc` | Normal mode |
| `dd` / `yy` / `p` | Delete / yank / paste line |
| `gg` / `G` | Top / bottom of buffer |
| `v` | Visual mode |
| `:w` | Submit message |
| `:q` | Quit Archon |

Full reference: `/keybindings`.

---

## Cost, Effort & Fast Mode

### Cost tracking

Per-turn token costs are accumulated per session and surfaced via `/cost` and `/usage`. Alerts:

```toml
[cost]
warn_threshold = 100.0   # Warn when session cost exceeds $N
hard_limit = 0.0         # 0.0 = no hard cap
```

### Effort levels

Affects `thinking_budget`, temperature, and context window allocation:

- `high`, full thinking budget, maximum context
- `medium`, reduced thinking, balanced
- `low`, minimal thinking, fastest responses

Switch at runtime: `/effort high`, `--effort low`, or `[api].default_effort`.

### Fast mode

```bash
archon --fast
# or /fast in TUI
```

Disables extended thinking, uses aggressive token limits, and skips some memory injection. Same model, lower quality, lower latency, good for quick questions.

---

## Context Compaction

Automatic compaction prevents hitting the context window limit.

```toml
[context]
compact_threshold = 0.8       # Fill % that triggers compaction
preserve_recent_turns = 3     # Always keep last N turns verbatim
prompt_cache = true           # Enable Anthropic prompt cache
```

### Manual compaction

| Command | Action |
|---------|--------|
| `/compact` | Auto-compact (LLM summarizes older turns) |
| `/compact micro` | Minimal compaction (preserve more detail) |
| `/compact snip N-M` | Remove turns N through M |
| `/compact auto` | Adaptive compression |

`PreCompact` / `PostCompact` hooks fire around compaction.

---

## Pipeline Engine

Archon includes two full agent pipelines ported from the TypeScript god-agent SDK to native Rust.

### Coding Pipeline (50 agents)

A 6-phase, 50-agent software development pipeline with runtime-loaded agent definitions (.md frontmatter + TOML manifests), gate enforcement, session recovery, and structured artefacts. Each agent receives an 11-layer composite prompt assembled from task analysis, agent instructions (.md body), codebase context, LEANN search results, and prior agent outputs.

```mermaid
graph LR
    subgraph P1["Phase 1: Understanding"]
        CA2["contract-agent"] --> RE["requirement-extractor"]
        RE --> RP["requirement-prioritizer"]
        RP --> SD3["scope-definer"]
        SD3 --> CG["context-gatherer"]
        CG --> PE["pattern-explorer"]
        PE --> TS["technology-scout"]
        TS --> CBA["codebase-analyzer"]
    end

    subgraph P2["Phase 2: Design"]
        FA["feasibility-analyzer"] --> RPL["research-planner"]
        RPL --> SD["system-designer"]
        SD --> CD["component-designer"]
        CD --> ID["interface-designer"]
        ID --> DA["data-architect"]
        DA --> PA["performance-architect"]
        PA --> SA["security-architect"]
    end

    subgraph P3["Phase 3: Wiring Plan"]
        IA["integration-architect"] --> WO["wiring-obligation-agent"]
    end

    subgraph P4["Phase 4: Implementation"]
        CG2["code-generator"] --> TI["type-implementer"]
        TI --> UI["unit-implementer"]
        UI --> SI["service-implementer"]
        SI --> DL["data-layer-implementer"]
        DL --> API["api-implementer"]
        API --> FI["frontend-implementer"]
        FI --> EH["error-handler-implementer"]
        EH --> CI["config-implementer"]
        CI --> LI["logger-implementer"]
        LI --> IVA["integration-verification-agent"]
    end

    subgraph P5["Phase 5: Testing"]
        TG["test-generator"] --> TR["test-runner"]
        TR --> IT["integration-tester"]
        IT --> RT["regression-tester"]
        RT --> ST["security-tester"]
        ST --> COV["coverage-analyzer"]
        COV --> TF["test-fixer"]
    end

    subgraph P6["Phase 6: Refinement"]
        PO["performance-optimizer"] --> CQI["code-quality-improver"]
        CQI --> FR["final-refactorer"]
        FR --> DM["dependency-manager"]
        DM --> IC["implementation-coordinator"]
        IC --> QG["quality-gate"]
        QG --> SO["sign-off-approver"]
    end

    P1 --> P2 --> P3 --> P4 --> P5 --> P6

    style P1 fill:#0f3460,stroke:#533483,color:#e0e0e0
    style P2 fill:#16213e,stroke:#533483,color:#e0e0e0
    style P3 fill:#1a1a2e,stroke:#533483,color:#e0e0e0
    style P4 fill:#0f3460,stroke:#e94560,color:#e0e0e0
    style P5 fill:#16213e,stroke:#e94560,color:#e0e0e0
    style P6 fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
```

NOTE: Each phase also has a phase-N-reviewer (Sherlock adversarial gate) and a recovery-agent in Phase 6, but they are omitted from the diagram for clarity.

**Prompt Assembly (11 layers):**

| Layer | Name | Priority | Source |
|-------|------|----------|--------|
| L1 | `base_prompt` | Required | Agent role, phase, model, description |
| L1.5 | `agent_instructions` | AgentInstructions | Full .md file body (parsed via frontmatter) |
| L2 | `task_context` | Required | User's task description |
| L3 | `leann_semantic_context` | LeannSemanticContext | LEANN code search results |
| L4 | `rlm_namespace_context` | RlmContext | Prior agent outputs from RLM store |
| L5 | `desc_episodes` | DescEpisodes | DESC episodic memory |
| L6 | `sona_patterns` | SonaPatterns | SONA trajectory patterns |
| L7 | `reflexion_trajectories` | ReflexionTrajectories | Failed trajectory injection for retries |
| L8 | `pattern_matcher_results` | PatternMatcherResults | Reasoning context |
| L9 | `sherlock_verdicts` | SherlockVerdicts | (reserved) |
| L10 | `algorithm_strategy` | AlgorithmStrategy | Algorithm-specific prompt snippet |
| L11 | `prompt_cap` | — | Token budget enforcement via truncation |

### Research Pipeline (46 agents)

A 7-phase, 46-agent PhD-level research pipeline with runtime-loaded agent definitions. Organized from Foundation through Validation with phases 6-7 having full tool access for writing output.

```mermaid
graph LR
    subgraph P1["Phase 1: Foundation"]
        SB["step-back-analyzer"] --> SD2["self-ask-decomposer"]
        SD2 --> AC["ambiguity-clarifier"]
        AC --> RPL["research-planner"]
        RPL --> CDef["construct-definer"]
        CDef --> DA2["dissertation-architect"]
        DA2 --> CS["chapter-synthesizer"]
    end

    subgraph P2["Phase 2: Discovery"]
        LM["literature-mapper"] --> STC["source-tier-classifier"]
        STC --> CE["citation-extractor"]
        CE --> CTM["context-tier-manager"]
    end

    subgraph P3["Phase 3: Architecture"]
        TFA["theoretical-framework-analyst"] --> CA["contradiction-analyzer"]
        CA --> GH["gap-hunter"]
        GH --> RA["risk-analyst"]
    end

    subgraph P4["Phase 4: Synthesis"]
        ES["evidence-synthesizer"] --> PAN["pattern-analyst"]
        PAN --> TS2["thematic-synthesizer"]
        TS2 --> TB2["theory-builder"]
        TB2 --> OI["opportunity-identifier"]
    end

    subgraph P5["Phase 5: Design"]
        MD["method-designer"] --> HG["hypothesis-generator"]
        HG --> MA["model-architect"]
        MA --> AP["analysis-planner"]
        AP --> SS["sampling-strategist"]
        SS --> IND["instrument-developer"]
        IND --> VG["validity-guardian"]
        VG --> MSC["methodology-scanner"]
        MSC --> MW["methodology-writer"]
    end

    subgraph P6["Phase 6: Writing"]
        IW["introduction-writer"] --> LRW["literature-review-writer"]
        LRW --> RW["results-writer"]
        RW --> DW["discussion-writer"]
        DW --> CW["conclusion-writer"]
        CW --> AW["abstract-writer"]
    end

    subgraph P7["Phase 7: Validation"]
        SR["systematic-reviewer"] --> ER["ethics-reviewer"]
        ER --> ADR["adversarial-reviewer"]
        ADR --> CQ["confidence-quantifier"]
        CQ --> CIV["citation-validator"]
        CIV --> RC["reproducibility-checker"]
        RC --> APA["apa-citation-specialist"]
        APA --> CON["consistency-validator"]
        CON --> QAS["quality-assessor"]
        QAS --> BD["bias-detector"]
        BD --> FLM["file-length-manager"]
    end

    P1 --> P2 --> P3 --> P4 --> P5 --> P6 --> P7

    style P1 fill:#0f3460,stroke:#533483,color:#e0e0e0
    style P2 fill:#16213e,stroke:#533483,color:#e0e0e0
    style P3 fill:#1a1a2e,stroke:#533483,color:#e0e0e0
    style P4 fill:#0f3460,stroke:#e94560,color:#e0e0e0
    style P5 fill:#16213e,stroke:#e94560,color:#e0e0e0
    style P6 fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
    style P7 fill:#0a0a1a,stroke:#e94560,color:#e0e0e0
```

### Pipeline Execution

Both pipelines share the `PipelineFacade` trait and a common runner loop:

```mermaid
graph TD
    INIT["Pipeline Init<br/>task + config"] --> LEANN_IDX["LEANN: Index Repository"]
    LEANN_IDX --> LOOP{"Next Agent?"}
    LOOP -- Yes --> PROMPT["Assemble Agent Prompt"]
    PROMPT --> LEANN_SEARCH["LEANN: Search for Context"]
    LEANN_SEARCH --> LLM["LLM Call<br/>(streaming)"]
    LLM --> QUALITY{"Quality Gate<br/>L-Score check"}
    QUALITY -- Pass --> STORE["Store Output"]
    QUALITY -- Fail --> RETRY{"Retries left?"}
    RETRY -- Yes --> REFLEXION["Inject Reflexion<br/>(failed trajectory)"]
    REFLEXION --> LLM
    RETRY -- No --> STORE
    STORE --> LEARN["Learning Feedback<br/>(SONA, AutoCapture)"]
    LEARN --> INDEX["LEANN: Index Modified Files"]
    INDEX --> LOOP
    LOOP -- No --> DONE["Pipeline Complete"]

    style INIT fill:#0f3460,stroke:#533483,color:#e0e0e0
    style LEANN_IDX fill:#1a1a2e,stroke:#533483,color:#e0e0e0
    style LLM fill:#16213e,stroke:#e94560,color:#e0e0e0
    style QUALITY fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
    style REFLEXION fill:#1a0a2e,stroke:#e94560,color:#e0e0e0
    style DONE fill:#0f3460,stroke:#533483,color:#e0e0e0
```

### Agent Definition System

Agent behavior is defined in `.archon/agents/` markdown files with YAML frontmatter and loaded at runtime by the `agent_loader` module. Execution order is governed by TOML manifests.

```
.archon/agents/
├── coding-pipeline/
│   ├── pipeline.toml          # 50-agent execution order + phase defs
│   ├── contract-agent.md      # Agent #1 (YAML frontmatter + instructions)
│   ├── requirement-extractor.md
│   ├── ...
│   └── recovery-agent.md      # Agent #50
└── phdresearch/
    ├── pipeline.toml          # 46-agent execution order + phase defs
    ├── step-back-analyzer.md
    ├── ...
    └── file-length-manager.md
```

**Frontmatter fields** (parsed by `agent_loader::parse_frontmatter`):

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Agent key (kebab-case) |
| `type` | string | Phase name |
| `description` | string | Role description |
| `algorithm` | string | Primary reasoning algorithm (ReAct, ToT, Reflexion) |
| `fallback_algorithm` | string | Fallback if primary fails |
| `memory_reads` | list | RLM namespaces to read |
| `memory_writes` | list | RLM namespaces to write |
| `tools` | list | Allowed tools |
| `qualityGates` | list/map | Quality gate criteria |
| `capabilities` | list | Agent capabilities |

### Gate Enforcement

Five deterministic gates enforce code quality using tool output only (no LLM self-assessment):

| Gate | Module | Description |
|------|--------|-------------|
| ForbiddenPatternScanner | `coding/gates.rs` | Blocks TODO, stubs, `unimplemented!()`, empty function bodies |
| CompilationGate | `coding/gates.rs` | `cargo build` / `npm run build` must exit 0 |
| OrphanDetectionGate | `coding/gates.rs` | Every new file must be referenced by at least one other file |
| TestsRunGate | `coding/gates.rs` | Test suite must exit 0 |
| E2ESmokeTestGate | `coding/gates.rs` | Feature invoked end-to-end with fraud detection (rejects test-only output) |

### Structured Artefacts

Six typed artefacts form the pipeline's audit chain:

```
TaskContract → EvidencePack → WiringPlan → ImplementationReport → ValidationReport → MergePacket
```

| Artefact | Producer | Contents |
|----------|----------|----------|
| `TaskContract` | contract-agent | Parsed intent, acceptance criteria, constraints |
| `EvidencePack` | Phase reviewers | File-line facts, call graphs, test references |
| `WiringPlan` | wiring-obligation-agent | Typed obligations that gate Phase 4 |
| `ImplementationReport` | implementation-coordinator | Changed files, new symbols, wiring status |
| `ValidationReport` | quality-gate | Gate results, AC trace with evidence |
| `MergePacket` | sign-off-approver | Risk report, evidence bundle, sign-off |

All artefacts are persisted atomically (write-to-tmp + rename) via `artefacts::save_artefact()`.

### Session Recovery

Pipeline sessions checkpoint after every agent completion. Interrupted sessions can be detected and resumed:

| Function | Description |
|----------|-------------|
| `checkpoint()` | Atomic write of session state (fsync + rename) |
| `resume()` | Reload interrupted session, reset to Running |
| `detect_interrupted()` | Find all Running/Paused sessions |
| `abort()` | Mark session as permanently failed |

### Ledger System

Three append-only ledgers provide a complete audit trail:

| Ledger | Records |
|--------|---------|
| `DecisionLedger` | All decisions with reason, affected files, timestamp |
| `TaskLedger` | Task assignments, status changes, wiring obligations |
| `VerificationLedger` | Gate pass/fail results with evidence summaries |

---

## LEANN Semantic Code Search

LEANN (Learning-Enhanced Approximate Nearest Neighbors) is a native semantic code search engine built into Archon. It indexes source code at the chunk level using tree-sitter parsing and vector embeddings, enabling natural-language queries over codebases.

```mermaid
graph TD
    subgraph indexing["Indexing Pipeline"]
        SRC["Source Files"] --> TS["Tree-Sitter<br/>Language Parser"]
        TS --> CHUNK["Chunker<br/>(function/class boundaries)"]
        CHUNK --> EMBED["Embedding<br/>(fastembed or OpenAI)"]
        EMBED --> STORE["CozoDB + HNSW Index"]
    end

    subgraph search["Search Pipeline"]
        QUERY["Natural Language Query"] --> QEMBED["Query Embedding"]
        QEMBED --> HNSW["HNSW Cosine Search"]
        HNSW --> RANK["Rank + Filter"]
        RANK --> RESULTS["SearchResult[]<br/>(file, lines, score, content)"]
    end

    subgraph queue["Background Queue"]
        WATCH["File Change Events"] --> QUEUE["Queue Processor"]
        QUEUE --> STORE
    end

    STORE --> HNSW

    style indexing fill:#0f3460,stroke:#533483,color:#e0e0e0
    style search fill:#16213e,stroke:#e94560,color:#e0e0e0
    style queue fill:#1a1a2e,stroke:#533483,color:#e0e0e0
```

### Features

| Feature | Details |
|---------|---------|
| Chunking | Tree-sitter AST-aware boundaries (functions, classes, methods) |
| Embeddings | fastembed local (768-dim BGE-base) or OpenAI (1536-dim) |
| Index | HNSW approximate nearest neighbors via CozoDB |
| Search | Cosine similarity with configurable top-k |
| Queue | Background indexing queue with add/process/status |
| Languages | Rust, Python, TypeScript, JavaScript, Go, Java, C, C++ |

### API

```rust
let index = CodeIndex::new("./index.db", embedding_config)?;

// Index a repository
index.index_repository(path, &config).await?;

// Search with natural language
let results = index.search_code("authentication middleware", 10)?;

// Find similar code
let similar = index.find_similar_code(snippet, 5)?;

// Background queue
index.add_to_queue(queue_path, &file_paths)?;
index.process_queue(queue_path).await?;
```

---

## Knowledge Base

CozoDB-backed document knowledge base with LLM compilation and Q&A.

```mermaid
graph TD
    subgraph ingest["Document Ingestion"]
        DOC["Documents<br/>(markdown, text, code)"] --> PARSE["Parse + Chunk"]
        PARSE --> EMBED2["Embed Chunks"]
        EMBED2 --> KB["CozoDB<br/>Knowledge Graph"]
    end

    subgraph compile["LLM Compilation"]
        KB --> SELECT["Select Nodes<br/>(by topic/type)"]
        SELECT --> LLM2["LLM Synthesis"]
        LLM2 --> COMPILED["Compiled Knowledge<br/>(structured summaries)"]
        COMPILED --> KB
    end

    subgraph qa["Q&A Pipeline"]
        Q["User Question"] --> SEARCH["Vector Search<br/>+ Graph Context"]
        SEARCH --> SYNTH["LLM Synthesize Answer"]
        SYNTH --> ANS["Answer<br/>(with citations)"]
        ANS --> |"file_answer=true"| KB
    end

    KB --> SEARCH

    style ingest fill:#0f3460,stroke:#533483,color:#e0e0e0
    style compile fill:#16213e,stroke:#533483,color:#e0e0e0
    style qa fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
```

### Q&A Scoring

Answer-type nodes receive a 0.9x relevance penalty to prevent answer recycling (answers to prior questions outranking source material). This is enforced by `EC-PIPE-018`.

### CLI

```bash
archon kb ingest ./docs/           # Ingest all documents in directory
archon kb query "How does auth work?"  # Q&A with citations
archon kb compile --topic security     # LLM-compile nodes by topic
archon kb stats                        # Index statistics
```

---

## Learning Systems

Archon's pipeline engine includes 8 interconnected learning systems that provide trajectory optimization, causal reasoning, graph neural enhancement, and contradiction detection.

```mermaid
graph TD
    subgraph core["Core Learning"]
        SONA["SONA Engine<br/>Trajectory-aware optimization"]
        RB["ReasoningBank<br/>4 core + 8 extended modes"]
        GNN["GNN Enhancer<br/>3-layer graph attention<br/>1536D → 1024D"]
    end

    subgraph memory_layer["Memory & Causality"]
        CM["CausalMemory<br/>Hypergraph with BFS"]
        PS["ProvenanceStore<br/>L-Scores, citation chains"]
        DESC["DESC<br/>Episode store"]
    end

    subgraph validation["Validation & Recovery"]
        SVS["ShadowVectorSearch<br/>Contradiction detection"]
        REF["Reflexion<br/>Failed trajectory injection"]
        AC["AutoCapture<br/>Pattern-based memory extraction"]
    end

    SONA --> |"trajectory feedback"| RB
    RB --> |"contextual reasoning"| GNN
    GNN --> |"enhanced embeddings"| RB
    RB --> |"causal queries"| CM
    CM --> |"cause/effect chains"| PS
    PS --> |"L-Scores"| SONA
    SVS --> |"contradiction reports"| RB
    REF --> |"failed trajectories"| SONA
    AC --> |"captured memories"| DESC

    style core fill:#0f3460,stroke:#e94560,color:#e0e0e0
    style memory_layer fill:#16213e,stroke:#533483,color:#e0e0e0
    style validation fill:#1a1a2e,stroke:#e94560,color:#e0e0e0
```

### System Details

| System | Purpose | Key Feature |
|--------|---------|-------------|
| **SONA** | Trajectory-aware optimization | Tracks agent performance across runs, adjusts prompts |
| **ReasoningBank** | Multi-modal reasoning (12 modes) | Core: deductive, inductive, abductive, analogical. Extended: adversarial, counterfactual, temporal, constraint, decomposition, first-principles, causal, contextual |
| **GNN Enhancer** | Graph neural network | 3-layer attention (1536->1280->1280->1024), Xavier init, NaN fallback, Adam optimizer, contrastive loss, EWC regularization |
| **CausalMemory** | Causal relationship tracking | Hypergraph with multi-cause hyperedges, BFS traversal (max 5 hops), cycle detection |
| **ProvenanceStore** | Source credibility scoring | L-Scores (recency decay, authority, corroboration, domain relevance), citation path traversal |
| **ShadowVectorSearch** | Contradiction detection | Semantic inversion (negate embedding), find docs similar to shadow vector |
| **DESC** | Episode store | Stores agent execution episodes for experience replay |
| **Reflexion** | Retry enhancement | Injects failed trajectory context into retry prompts (attempt > 1 only) |

### Graceful Degradation

All learning systems accept `Option<T>` dependencies. When a system is unavailable (e.g., no GNN weights trained yet), the pipeline continues with reduced capability rather than failing. This satisfies `REQ-LEARN-013`.

### GNN Architecture

```mermaid
graph LR
    INPUT["Input<br/>1536-dim"] --> L1["Layer 1<br/>Attention + ReLU<br/>1536 → 1280"]
    L1 --> L2["Layer 2<br/>Attention + ReLU<br/>1280 → 1280"]
    L2 --> L3["Layer 3<br/>Attention + Tanh<br/>1280 → 1024"]
    L3 --> OUTPUT["Output<br/>1024-dim"]

    L1 --> |"cache"| CACHE["LRU Cache<br/>1000 entries, 300s TTL"]
    L2 --> |"cache"| CACHE
    L3 --> |"cache"| CACHE

    subgraph training["Training Loop"]
        LOSS["Contrastive Loss<br/>(triplet mining)"]
        ADAM["Adam Optimizer<br/>β1=0.9, β2=0.999"]
        EWC["EWC Regularizer<br/>(Fisher information)"]
        LOSS --> ADAM --> EWC
    end

    OUTPUT --> LOSS

    style INPUT fill:#0f3460,stroke:#533483,color:#e0e0e0
    style OUTPUT fill:#0f3460,stroke:#533483,color:#e0e0e0
    style L1 fill:#16213e,stroke:#e94560,color:#e0e0e0
    style L2 fill:#16213e,stroke:#e94560,color:#e0e0e0
    style L3 fill:#16213e,stroke:#e94560,color:#e0e0e0
    style training fill:#1a1a2e,stroke:#533483,color:#e0e0e0
    style CACHE fill:#1a1a2e,stroke:#533483,color:#e0e0e0
```

### AutoCapture

Regex-based pattern detection that extracts memories from conversation without LLM inference:

| Pattern Type | Examples | Confidence |
|-------------|----------|------------|
| Correction | "no", "wrong", "that's incorrect" | 0.8 |
| Decision | "let's go with", "we decided" | 0.7 |
| Error | "failed", "crashed", "broken" | 0.75 |
| Preference | "I prefer", "always use", "never do" | 0.7 |
| Project State | "deployed", "released", "merged" | 0.65 |

Deduplication uses Jaccard similarity with a 0.8 threshold to prevent redundant captures.

---

## Crate Architecture

```
archon (binary)
│
├── archon-core          Agent loop, config, skills, hooks, CLI parsing
│   ├── orchestrator/    Multi-agent team execution (seq/parallel/pipeline/dag)
│   ├── skills/          Builtin + expanded slash commands
│   ├── hooks/           Lifecycle hook executor
│   └── team/            Team backend state
│
├── archon-llm           Claude API client (streaming SSE, retries, OAuth)
│   ├── anthropic.rs     Messages API client
│   ├── identity.rs      9-layer spoofing
│   ├── oauth.rs         PKCE flow + token refresh (file-locked)
│   ├── effort.rs        Effort level → API param mapping
│   └── fast_mode.rs     Fast mode overrides
│
├── archon-tools         40+ tools
│   ├── bash/read/write/edit/glob/grep    File & shell
│   ├── agent_tool.rs                     Subagent spawn
│   ├── lsp_client.rs + lsp_tool.rs       LSP bridge
│   ├── webfetch.rs                       HTTP fetch + HTML parse
│   ├── web_search.rs                     DuckDuckGo web search
│   ├── task_*.rs                         Background task tools
│   ├── cron_*.rs                         Scheduling
│   ├── team_*.rs                         Team coordination
│   ├── mcp_resources.rs                  MCP bridge tools
│   ├── checkpoint.rs                     File snapshots
│   └── toolsearch.rs                     Dynamic tool discovery
│
├── archon-permissions   Permission mode enforcement + rule engine
│
├── archon-mcp           MCP transport (stdio/ws/http-streamable)
│   ├── transport_ws.rs  WebSocket with backoff + sleep detection
│   └── config.rs        .mcp.json parsing + env expansion
│
├── archon-consciousness System prompt assembly + cross-session learning
│   ├── personality.rs   MBTI/Enneagram → prompt fragment
│   ├── rules.rs         RulesEngine (scored rules, decay, reinforcement)
│   ├── corrections.rs   CorrectionTracker (5 severity levels, auto-detect)
│   ├── persistence.rs   Cross-session snapshots, trends, briefing
│   ├── defaults.rs      Idempotent rule seeding
│   ├── inner_voice.rs   Confidence, energy, struggles, successes
│   └── assembler.rs     System prompt assembly (7 sources)
│
├── archon-memory        CozoDB memory graph + consolidation
│   ├── graph.rs         store / recall / search / relationships
│   ├── injection.rs     Per-turn <memories> block
│   ├── extraction.rs    Auto-extraction pipeline
│   ├── garden.rs        6-phase consolidation, briefing, /garden command
│   ├── embedding/       fastembed local embeddings (BGE-base-en-v1.5)
│   ├── vector_search.rs HNSW cosine similarity
│   └── hybrid_search.rs BM25 + vector cosine (alpha-blended)
│
├── archon-session       Session + checkpoint + plan persistence
│   ├── storage.rs       Session save/load/list/prefix-match
│   ├── resume.rs        4-step ID+name resolution
│   ├── checkpoint.rs    File snapshot store (diff, restore-to-turn)
│   └── plan.rs          Plan storage (PlanDocument, PlanStore)
│
├── archon-tui           ratatui TUI
│   ├── app.rs           State, events, input box
│   ├── split_pane.rs    Split-pane layout (Ctrl+\)
│   ├── theme.rs         22 themes
│   └── vim.rs           Vim mode keybindings
│
├── archon-pipeline      50-agent coding + 46-agent research pipelines
│   ├── coding/          CodingFacade, 50 agent definitions, 11-layer prompt
│   │   ├── agents.rs    Static AGENTS array (50 agents, 6 phases)
│   │   ├── facade.rs    CodingFacade with agent_instructions layer
│   │   ├── gates.rs     5 deterministic gates + fraud detection
│   │   ├── contract.rs  TaskContract artefact
│   │   ├── evidence.rs  EvidencePack + validation
│   │   └── wiring.rs    WiringPlan obligations
│   ├── research/        ResearchFacade, 46 agent definitions, 7 phases
│   ├── agent_loader.rs  Runtime .md frontmatter parser
│   ├── manifest.rs      TOML pipeline manifest parser
│   ├── artefacts.rs     6 typed artefacts + AC traced gate
│   ├── session.rs       Checkpoint / resume / detect_interrupted
│   ├── ledgers.rs       Append-only Decision/Task/Verification ledgers
│   ├── layered_context.rs  L0-L3 four-tier context loader
│   ├── runner.rs        PipelineFacade trait, shared runner loop
│   ├── prompt_cap.rs    Token budget enforcement (12 priority tiers)
│   ├── retry.rs         Retry with Reflexion injection
│   ├── kb/              Knowledge base (ingest, compile, query, Q&A)
│   ├── learning/        SONA, ReasoningBank, GNN, CausalMemory, Provenance,
│   │                    ShadowVectorSearch, DESC, Reflexion, extended modes
│   └── capture.rs       AutoCapture (regex-based memory extraction)
│
├── archon-leann         Native semantic code search
│   ├── indexer.rs       Repository/file indexing with embeddings
│   ├── chunker.rs       Tree-sitter AST-aware code chunking
│   ├── search.rs        HNSW cosine similarity search
│   ├── queue.rs         Background indexing queue processor
│   └── metadata.rs      CodeChunk, SearchResult, IndexConfig types
│
├── archon-context       Context compaction engine
│
├── archon-plugin        Dynamic plugin loader
│   ├── loader.rs        .so/.dll/.dylib loading
│   ├── host.rs          JSON-RPC plugin host
│   └── adapter_*.rs     Tool/hook/skill/command adapters
│
└── archon-sdk           Embedding SDK + Web/IDE bridges
    ├── builder.rs       AgentBuilder for embedding
    ├── web/             WebSocket server + HTTP
    └── ide/             IDE protocol handlers
```

### Key Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `ratatui` | 0.29 | Terminal UI rendering |
| `crossterm` | 0.28 | Cross-platform terminal backend |
| `cozo-ce` | 0.7.13 | CozoDB, memory/session/checkpoint store |
| `fastembed` | 4 | Local vector embeddings (no API) |
| `tokio` | 1 | Async runtime |
| `reqwest` | 0.12 | HTTP client (rustls) |
| `clap` | 4 | CLI argument parsing |
| `serde` / `toml` |, | Config serialization |
| `git2` | 0.19 | Git branch detection |
| `tree-sitter` | 0.24 | Syntax highlighting + LEANN code chunking |
| `async-lsp` |, | LSP protocol client |
| `tungstenite` |, | WebSocket transport |
| `tower-http` | 0.6 | HTTP middleware (web UI) |

---

## Phase Roadmap

| Phase | Status | Description |
|-------|--------|-------------|
| **Phase 1**, Core Engine | ✅ Complete | Agent loop, streaming API, tool execution, permission system, config, TUI, session management |
| **Phase 2**, Consciousness | ✅ Complete | Memory graph (CozoDB), auto-extraction, per-turn injection, rules engine, personality config, inner voice |
| **Phase 3**, UX & Ergonomics | ✅ Complete | 22 themes, MBTI themes, resume by name/prefix, `/color` and `/theme` commands, memory tools wired |
| **Phase 4**, Plugins & Skills | ✅ Complete | Plugin system (`archon-plugin`), user-defined slash commands, skill system, hook extensibility |
| **Phase 5**, Multi-Agent & Learning | ✅ Complete | Subagent orchestration, team execution, MCP transport, LSP client, WebSocket remote, personality persistence (cross-session InnerVoice + rule scores + trends), memory garden (6-phase consolidation + `/garden` command), correction tracking with severity-based rule reinforcement |
| **Phase 6**, Pipelines & Intelligence | ✅ Complete | 50-agent coding pipeline (CodingFacade, 11-layer prompt, 6 phases), agent_loader (.md frontmatter + TOML manifests), 5 deterministic gates, 6 structured artefacts, session recovery, append-only ledgers, layered context (L0-L3), 46-agent research pipeline (ResearchFacade, 7 phases), LEANN semantic code search (tree-sitter + HNSW), Knowledge Base (ingest/compile/Q&A), Learning systems (SONA, ReasoningBank 12 modes, GNN 3-layer attention, CausalMemory hypergraph, ProvenanceStore L-Scores, ShadowVectorSearch contradiction detection, DESC, Reflexion), AutoCapture |

---

## License

See `LICENSE` file. Archon is not affiliated with Anthropic.

> **Note:** Archon proxies the Anthropic Claude API. You must have a valid API key and comply with Anthropic's usage policies.
