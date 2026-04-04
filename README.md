# Archon CLI

A privacy-first, self-aware AI coding assistant written in Rust. Archon replaces cloud-dependent AI CLIs with a fully local consciousness layer — persistent memory, configurable personality, behavioral rules, and an interactive TUI — while proxying Claude's API directly with zero telemetry.

---

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Setup](#setup)
  - [macOS](#macos)
  - [Linux](#linux)
  - [Windows](#windows)
- [Configuration](#configuration)
- [Slash Commands](#slash-commands)
- [Themes](#themes)
- [Memory System](#memory-system)
- [Consciousness System](#consciousness-system)
- [Agent Loop](#agent-loop)
- [Subagent Spawning](#subagent-spawning)
- [Session Management](#session-management)
- [Crate Architecture](#crate-architecture)
- [Phase Roadmap](#phase-roadmap)

---

## Overview

| Feature | Claude Code | Archon |
|---------|-------------|--------|
| Telemetry | Yes | None |
| Memory | Cloud | Local CozoDB graph |
| Personality | Fixed | Configurable (MBTI, Enneagram, traits) |
| Behavioral rules | None | User-defined, persisted |
| TUI | Basic | Full ratatui TUI with 22 themes |
| Session resume | ID only | ID prefix, name, or name prefix |
| Tool execution | Node.js | Native Rust async |
| Binary size | ~200 MB | ~15 MB (release, stripped) |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        archon (binary)                       │
│                                                              │
│  ┌───────────┐  ┌────────────────┐  ┌─────────────────────┐ │
│  │ archon-tui│  │  archon-core   │  │ archon-consciousness │ │
│  │  (ratatui)│  │ agent / tools  │  │ rules / personality  │ │
│  └─────┬─────┘  └───────┬────────┘  └──────────┬──────────┘ │
│        │                │                        │            │
│        └────────────────┴────────────────────────┘            │
│                         │                                      │
│  ┌──────────────┐  ┌────┴──────────┐  ┌────────────────────┐ │
│  │archon-session│  │ archon-memory │  │    archon-llm      │ │
│  │ (CozoDB)     │  │ (CozoDB graph)│  │ (Claude API proxy) │ │
│  └──────────────┘  └───────────────┘  └────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
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
git clone https://github.com/your-org/archon-cli
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

**Optional — brew dependencies** (only needed if build fails due to OpenSSL):
```bash
brew install pkg-config openssl
export PKG_CONFIG_PATH="$(brew --prefix openssl)/lib/pkgconfig"
```

---

### Linux

#### Ubuntu / Debian

```bash
# 1. Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 2. Install build dependencies
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev

# 3. Clone and build
git clone https://github.com/your-org/archon-cli
cd archon-cli
cargo build --release

# 4. Install to PATH
sudo cp target/release/archon /usr/local/bin/archon
# or
cargo install --path .

# 5. Set API key
export ANTHROPIC_API_KEY="sk-ant-..."
echo 'export ANTHROPIC_API_KEY="sk-ant-..."' >> ~/.bashrc
```

#### Fedora / RHEL / Rocky

```bash
sudo dnf install -y gcc pkg-config openssl-devel
# Then follow steps 1, 3–5 above
```

#### Arch Linux

```bash
sudo pacman -S base-devel openssl pkg-config
# Then follow steps 1, 3–5 above
```

---

### Windows

#### Option A: Native (Windows 10/11)

```powershell
# 1. Install Rust via winget
winget install Rustlang.Rustup

# 2. Install Visual Studio C++ build tools (required)
winget install Microsoft.VisualStudio.2022.BuildTools
# Select "Desktop development with C++" during install

# 3. Open a new terminal and clone
git clone https://github.com/your-org/archon-cli
cd archon-cli
cargo build --release

# 4. Add to PATH (PowerShell profile or System Environment Variables)
$env:PATH += ";$PWD\target\release"
$env:ANTHROPIC_API_KEY = "sk-ant-..."

# 5. Run
.\target\release\archon.exe
```

#### Option B: WSL2 (Recommended for Windows)

WSL2 gives you a full Linux environment. Install Ubuntu from the Microsoft Store, then follow the [Linux instructions](#linux) above.

```powershell
# Enable WSL2
wsl --install -d Ubuntu
# Then inside the Ubuntu terminal:
# Follow Linux/Ubuntu setup above
```

---

## Configuration

Archon generates a commented config file on first run at `~/.config/archon/config.toml`.

```toml
[api]
default_model = "claude-sonnet-4-6"   # Model to use for the main agent
thinking_budget = 16384               # Max thinking tokens (extended thinking)
default_effort = "high"               # "low" | "medium" | "high"
max_retries = 3

[identity]
mode = "spoof"                        # "spoof" | "native"
spoof_version = "2.1.89"             # Reported as Claude Code version to the API
anti_distillation = false

[personality]
name = "Archon"                       # The assistant's name (shown in TUI header)
type = "INTJ"                         # MBTI type — auto-selects matching theme
enneagram = "4w5"
traits = ["strategic", "direct", "truth-over-comfort"]
communication_style = "terse"         # Injected into system prompt

[consciousness]
inner_voice = true                    # Enables background monologue before responses
energy_decay_rate = 0.02
initial_rules = [
    # These rules are seeded into the CozoDB rules graph on first run.
    # Adding a new rule to this list will add it on next startup (idempotent).
    "Always ask before modifying files",
    "Explain reasoning before acting",
    "Never create files unless explicitly requested",
]

[tools]
bash_timeout = 120                    # Seconds before bash tool times out
bash_max_output = 102400              # Max bytes captured from bash
max_concurrency = 4                   # Max parallel tool executions

[permissions]
mode = "ask"                          # "ask" | "auto" | "deny"
allow_paths = []                      # Always-allowed paths (bypass ask)
deny_paths = []                       # Always-denied paths

[memory]
enabled = true                        # Enable CozoDB memory graph

[context]
compact_threshold = 0.8               # Context fill % that triggers compaction
preserve_recent_turns = 3

[session]
auto_resume = true                    # Resume last session on startup

[logging]
level = "info"                        # "trace" | "debug" | "info" | "warn" | "error"
max_files = 50
max_file_size_mb = 10

[cost]
warn_threshold = 100.0               # Warn when session cost exceeds $N
hard_limit = 0.0                     # 0.0 = no hard limit
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `ANTHROPIC_API_KEY` | **Required.** Your Claude API key |
| `ARCHON_CONFIG` | Override config file path |
| `ARCHON_LOG` | Override log level |
| `RUST_LOG` | Tracing subscriber filter |

---

## Slash Commands

Commands entered in the TUI input box.

| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/clear` | Clear the chat history |
| `/exit` | Exit Archon |
| `/theme <name>` | Switch UI theme (see [Themes](#themes)) |
| `/color <name>` | Change accent color only |
| `/compact` | Trigger context compaction now |
| `/cost` | Show session cost breakdown |
| `/permissions` | Show current permission mode |
| `/model <name>` | Switch model mid-session |
| `/checkpoint` | Save a session checkpoint |
| `/resume` | Show resumable sessions |

---

## Themes

Switch with `/theme <name>`. All 22 themes:

### MBTI Themes

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

### Utility Themes

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

The memory system is the backbone of Archon. Unlike Claude Code which discards context between sessions, Archon persists facts, decisions, corrections, and patterns in a local CozoDB graph database.

```
┌─────────────────────────────────────────────────────────────────┐
│                    Memory Lifecycle                             │
│                                                                  │
│  TURN N                                                          │
│  ┌─────────┐    ┌──────────────────┐    ┌──────────────────┐   │
│  │  User   │───►│  MemoryInjector  │───►│  System Prompt   │   │
│  │ Message │    │  (per-turn recall│    │  <memories>      │   │
│  └─────────┘    │   from graph)    │    │  block injected  │   │
│                 └──────────────────┘    └──────────────────┘   │
│                                                  │               │
│                                                  ▼               │
│                                         ┌────────────────┐      │
│                                         │  Claude API    │      │
│                                         │  (response)    │      │
│                                         └───────┬────────┘      │
│                                                  │               │
│  Every N turns  ◄────────────────────────────────┘               │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  Auto-Extraction (background tokio task)                  │  │
│  │  LLM reads recent turns → extracts Facts/Decisions/       │  │
│  │  Preferences/Rules/Corrections/Patterns                   │  │
│  │  → store_memory() → CozoDB                               │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                   │
│  AGENT TOOLS (callable by LLM)                                   │
│  ┌──────────────────┐    ┌───────────────────────────────────┐  │
│  │  memory_store    │    │  memory_recall                    │  │
│  │  (explicit save) │    │  (semantic search by keyword)     │  │
│  └──────────────────┘    └───────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

### Memory Types

| Type | When Used |
|------|-----------|
| `Fact` | Objective information learned about the codebase or user |
| `Decision` | Architecture/design choices made |
| `Preference` | User preferences about style, tools, workflow |
| `Rule` | Behavioral constraints (e.g., "always ask before deleting") |
| `Correction` | Things the assistant got wrong and should not repeat |
| `Pattern` | Recurring code patterns observed in the project |

### Storage

- **Database**: CozoDB (Datalog, SQLite WAL backend)
- **Path**: `~/.local/share/archon/memory.db` (Linux/macOS) or `%APPDATA%\archon\memory.db` (Windows)
- **Embeddings**: fastembed (local, no network calls)
- **Search**: Hybrid — keyword BM25 + vector cosine similarity

---

## Consciousness System

The consciousness system shapes how the assistant presents itself and reasons through problems. It assembles the system prompt from multiple sources before each API call.

```
┌────────────────────────────────────────────────────────────────┐
│                  System Prompt Assembly                        │
│                                                                │
│  config.toml                                                   │
│  ┌────────────────────┐                                        │
│  │ [personality]      │──► name, MBTI, traits, style           │
│  │ [consciousness]    │──► inner_voice, initial_rules           │
│  └────────────────────┘          │                            │
│                                  ▼                            │
│  CozoDB rules graph              │                            │
│  ┌────────────────────┐          │                            │
│  │ RulesEngine        │──► <behavioral_rules> block            │
│  │ (persisted rules)  │          │                            │
│  └────────────────────┘          │                            │
│                                  ▼                            │
│  CozoDB memory graph             │                            │
│  ┌────────────────────┐          │                            │
│  │ MemoryInjector     │──► <memories> block (per-turn recall) │
│  │ (per-turn recall)  │          │                            │
│  └────────────────────┘          │                            │
│                                  ▼                            │
│  InnerVoice (if enabled)         │                            │
│  ┌────────────────────┐          │                            │
│  │ Background monolog │──► prepended to assistant response    │
│  └────────────────────┘          │                            │
│                                  ▼                            │
│                         ┌────────────────┐                    │
│                         │  Final System  │                    │
│                         │  Prompt sent   │                    │
│                         │  to Claude API │                    │
│                         └────────────────┘                    │
└────────────────────────────────────────────────────────────────┘
```

### Configuring Rules

Rules in `config.toml` under `[consciousness].initial_rules` are seeded into CozoDB on startup — idempotently. Adding a new rule to the list will inject it on next run without duplicating existing rules.

The LLM can also create rules dynamically using the `memory_store` tool with `memory_type = "Rule"`.

---

## Agent Loop

The main interactive loop that drives every conversation turn.

```
┌─────────────────────────────────────────────────────────────────┐
│                      Agent Loop                                 │
│                                                                  │
│   ┌──────────────────────────────────────────────────────────┐  │
│   │  1. User Input (TUI or stdin)                            │  │
│   └──────────────────────┬───────────────────────────────────┘  │
│                           │                                       │
│                           ▼                                       │
│   ┌──────────────────────────────────────────────────────────┐  │
│   │  2. Context Assembly                                     │  │
│   │     - inject behavioral_rules from RulesEngine           │  │
│   │     - inject memories from MemoryInjector                │  │
│   │     - attach conversation history                        │  │
│   └──────────────────────┬───────────────────────────────────┘  │
│                           │                                       │
│                           ▼                                       │
│   ┌──────────────────────────────────────────────────────────┐  │
│   │  3. Claude API Call (streaming SSE)                      │  │
│   │     - extended thinking if effort = "high"               │  │
│   │     - renders thinking dots in TUI while streaming       │  │
│   └──────────────────────┬───────────────────────────────────┘  │
│                           │                                       │
│            ┌──────────────┴──────────────┐                       │
│            │                             │                        │
│            ▼                             ▼                        │
│   ┌─────────────────┐        ┌─────────────────────────────┐    │
│   │  text response  │        │  tool_use block             │    │
│   │  → render TUI   │        │  → dispatch to tool handler │    │
│   └─────────────────┘        └──────────────┬──────────────┘    │
│                                              │                    │
│                                              ▼                    │
│                               ┌─────────────────────────────┐   │
│                               │  Tool Execution              │   │
│                               │  (Bash/Read/Write/Agent/...) │   │
│                               └──────────────┬──────────────┘   │
│                                              │                    │
│                                              ▼                    │
│                               ┌─────────────────────────────┐   │
│                               │  tool_result → append to    │   │
│                               │  conversation → loop back   │   │
│                               │  to step 3                  │   │
│                               └─────────────────────────────┘   │
│                                                                   │
│   After N turns: trigger_memory_extraction (background task)     │
└───────────────────────────────────────────────────────────────────┘
```

---

## Subagent Spawning

The `Agent` tool enables the main agent to spawn child agents for parallel or delegated work. Each subagent is a fully isolated `archon-core` agent instance with its own conversation context.

```
┌─────────────────────────────────────────────────────────────────┐
│                    Subagent Spawning                            │
│                                                                  │
│  Parent Agent (main loop)                                        │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  LLM emits: tool_use { name: "Agent", input: { ... } }  │   │
│  └──────────────────────────┬─────────────────────────────┘    │
│                              │                                    │
│                              ▼                                    │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  agent_tool.rs dispatches spawn                          │   │
│  │  Reads subagent_type → looks up skill/prompt template    │   │
│  └──────────────────┬──────────────────────────────────────┘    │
│                      │                                            │
│          ┌───────────┼───────────┐                               │
│          ▼           ▼           ▼                                │
│  ┌─────────────┐ ┌──────────┐ ┌──────────┐                      │
│  │  Child 1    │ │ Child 2  │ │ Child 3  │  (parallel if        │
│  │  archon-core│ │archon-   │ │archon-   │   max_concurrency    │
│  │  instance   │ │core inst.│ │core inst.│   allows)            │
│  └──────┬──────┘ └────┬─────┘ └────┬─────┘                      │
│         │              │             │                             │
│         └──────────────┴─────────────┘                           │
│                         │                                         │
│                         ▼                                         │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │  Results gathered → tool_result appended to parent ctx   │   │
│  │  Parent continues its agent loop with results in hand    │   │
│  └──────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

Subagents have access to the same tool set as the parent (Bash, Read, Write, Glob, Grep, etc.) but run in isolated task contexts managed by `archon-tools/src/task_manager.rs`.

---

## Session Management

Sessions are stored in CozoDB under `~/.local/share/archon/sessions.db`. Each session captures:

- Full message history
- Git branch at session start
- Working directory
- Token usage and cost
- Session name (auto-generated or user-provided)

### Resuming Sessions

```bash
# Resume by full UUID
archon --resume 8383f1ea-1234-5678-abcd-000000000000

# Resume by UUID prefix (unique match required)
archon --resume 8383f1ea

# Resume by exact session name
archon --resume "fix-auth-bug"

# Resume by session name prefix (unique match required)
archon --resume "fix-auth"

# List all sessions (from TUI: /resume)
archon --list-sessions
```

Resolution order: exact UUID → UUID prefix → exact name → name prefix. Ambiguous prefix matches return an error listing the candidates.

---

## Crate Architecture

```
archon (binary)
│
├── archon-core          Agent loop, config, skills, hooks, CLI parsing
│   ├── archon-llm       Claude API client (streaming SSE, retries)
│   ├── archon-tools     All tool implementations (Bash, Read, Write, Agent, ...)
│   │   └── memory.rs    MemoryStoreTool + MemoryRecallTool
│   ├── archon-permissions  Permission mode enforcement
│   └── archon-mcp       MCP server/client bridge
│
├── archon-consciousness  System prompt assembly
│   ├── personality.rs   PersonalityProfile → system prompt fragment
│   ├── rules.rs         RulesEngine (CozoDB-backed)
│   ├── defaults.rs      load_configured_defaults() — idempotent rule seeding
│   └── inner_voice.rs   Background monologue generator
│
├── archon-memory         CozoDB memory graph
│   ├── graph.rs          store_memory / recall_memories / search
│   ├── injection.rs      MemoryInjector — per-turn recall → <memories> block
│   ├── extraction.rs     Auto-extraction pipeline (background tokio task)
│   ├── embedding/        fastembed local embeddings
│   └── hybrid_search.rs  BM25 + vector cosine hybrid search
│
├── archon-session        Session persistence (CozoDB)
│   ├── storage.rs        SessionStore — save/load/list/prefix-match sessions
│   └── resume.rs         resume_session() — 4-step ID+name resolution
│
├── archon-tui            ratatui TUI
│   ├── app.rs            App state, TuiEvent enum, event loop
│   └── theme.rs          Theme struct, 22 themes, parse_color/theme_by_name
│
└── archon-context        Context compaction
```

### Key Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `ratatui` | 0.29 | Terminal UI rendering |
| `crossterm` | 0.28 | Cross-platform terminal backend |
| `cozo-ce` | 0.7.13 | CozoDB — memory graph + session store |
| `fastembed` | 4 | Local vector embeddings (no API) |
| `tokio` | 1 | Async runtime |
| `reqwest` | 0.12 | HTTP client for Claude API (rustls, no OpenSSL dep) |
| `clap` | 4 | CLI argument parsing |
| `serde` / `toml` | — | Config serialization |
| `git2` | 0.19 | Git branch detection for sessions |
| `tree-sitter` | 0.24 | Code syntax highlighting in TUI |

---

## Phase Roadmap

| Phase | Status | Description |
|-------|--------|-------------|
| **Phase 1** — Core Engine | ✅ Complete | Agent loop, streaming API, tool execution, permission system, config, TUI, session management |
| **Phase 2** — Consciousness | ✅ Complete | Memory graph (CozoDB), auto-extraction, per-turn injection, rules engine, personality config, inner voice, configurable initial rules |
| **Phase 3** — UX & Ergonomics | 🔄 In Progress | 22 themes, MBTI themes, resume by name/prefix, `/color` and `/theme` commands, memory + recall tools wired to LLM |
| **Phase 4** — Plugins & Skills | 🔜 Planned | Plugin system (`archon-plugin`), user-defined slash commands, skill marketplace, hook system extensibility |
| **Phase 5** — Multi-Agent | 🔜 Planned | Persistent subagent pool, cross-session memory sharing, agent specialization profiles, orchestration UI |

---

## License

See `LICENSE` file. Archon is not affiliated with Anthropic.

> **Note:** Archon proxies the Anthropic Claude API. You must have a valid API key and comply with Anthropic's usage policies.
