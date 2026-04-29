# CLI flags

Run `archon --help` for the live, authoritative listing. Every flag below is verified against `src/cli_args.rs`.

## Subcommands

| Subcommand | Synopsis |
|---|---|
| `archon` | Start interactive TUI (default) |
| `archon login` | Authenticate via OAuth PKCE flow |
| `archon logout` | Sign out (clears OAuth tokens) |
| `archon serve [--port PORT] [--token-path PATH]` | Start WebSocket server for remote agent access |
| `archon remote ws <URL> [--token TOKEN]` | Connect to remote agent via WebSocket |
| `archon remote ssh <TARGET>` | Connect to remote agent via SSH |
| `archon web [--port PORT] [--bind-address ADDR] [--no-open]` | Start browser-based web UI |
| `archon team run --team NAME <GOAL>` | Execute a multi-agent team on a goal |
| `archon team list` | List configured teams |
| `archon plugin list` | List discovered plugins |
| `archon plugin info <NAME>` | Show plugin details |
| `archon ide-stdio` | Run in IDE stdio mode (JSON-RPC over stdin/stdout) |
| `archon pipeline code <TASK> [--dry-run]` | Run the coding pipeline on a task |
| `archon pipeline research <TOPIC> [--dry-run]` | Run the research pipeline on a topic |
| `archon pipeline status <SESSION_ID>` | Show pipeline session status |
| `archon pipeline resume <SESSION_ID>` | Resume an interrupted pipeline session |
| `archon pipeline list` | List all pipeline sessions |
| `archon pipeline abort <SESSION_ID>` | Abort a running pipeline session |
| `archon pipeline run <FILE> [--format FMT] [--detach]` | Run declarative pipeline from spec file |
| `archon pipeline cancel <ID>` | Cancel a running declarative pipeline |
| `archon run-agent-async <NAME> [--input FILE] [--version REQ] [--detach]` | Submit an async agent task |
| `archon task-status <TASK_ID> [--watch]` | Check status of an async task |
| `archon task-result <TASK_ID> [--stream]` | Get result of a completed async task |
| `archon task-cancel <TASK_ID>` | Cancel a running async task |
| `archon task-list [--state STATE] [--agent AGENT] [--since DURATION]` | List async tasks |
| `archon task-events <TASK_ID> [--from-seq SEQ]` | Stream task events (NDJSON) |
| `archon metrics` | Prometheus task execution metrics |
| `archon agent-list [--include-invalid]` | List all discovered agents |
| `archon agent-search [--tag TAG] [--capability CAP] [--name-pattern P] [--version REQ]` | Search agents |
| `archon agent-info <NAME> [--version REQ] [--json]` | Show detailed agent information |
| `archon update [--check] [--force]` | Check for / apply updates |

## Top-level flags

### Mode and I/O

| Flag | Purpose |
|---|---|
| `-p, --print [QUERY]` | Non-interactive single-query mode (`-p` reads stdin) |
| `--input-format <FMT>` | `text` / `json` / `stream-json` (default: text) |
| `--output-format <FMT>` | `text` / `json` / `stream-json` (default: text) |
| `--json-schema <SCHEMA>` | Validate final assistant output against JSON schema |
| `--max-turns <N>` | Hard cap on agent turns |
| `--max-budget-usd <AMOUNT>` | Hard cost limit in USD |
| `--no-session-persistence` | Don't persist session to disk (print mode) |
| `--headless` | No TUI; JSON-lines stdio for backend integration |
| `--session-id <ID>` | Session ID for headless/remote (auto-generated if omitted) |

### Session management

| Flag | Purpose |
|---|---|
| `-n, --session-name <NAME>` | Assign name to new session |
| `-c, --continue-session` | Continue most recent session in cwd |
| `--fork-session` | Fork resumed session instead of appending |
| `--resume [ID\|NAME]` | Resume by ID, name, or prefix (list if no arg) |
| `--no-resume` | Disable auto-resume for this invocation |
| `--sessions [...]` | Session search & management |

### Model and behaviour

| Flag | Purpose |
|---|---|
| `--model <MODEL>` | Override default model |
| `--fast` | Fast mode (reduced latency, lower quality) |
| `--effort <LEVEL>` | `high` / `medium` / `low` |
| `--identity-spoof` | Enable Claude Code header spoofing |
| `--agent <NAME>` | Use named agent definition |
| `--system-prompt <TEXT>` / `--system-prompt-file <PATH>` | Replace system prompt |
| `--append-system-prompt <TEXT>` / `--append-system-prompt-file <PATH>` | Append to default system prompt |
| `--theme <NAME>` | Startup TUI theme |
| `--output-style <NAME>` | `Explanatory` / `Learning` / `Formal` / `Concise` |

### Permissions

| Flag | Purpose |
|---|---|
| `--permission-mode <MODE>` | Override permissions (`default`, `acceptEdits`, `plan`, `auto`, `dontAsk`, `bubble`, `bypassPermissions`) |
| `--dangerously-skip-permissions` | Skip all permission checks |
| `--allow-dangerously-skip-permissions` | Allow `bypassPermissions` in mode cycle |
| `--bare` | Minimal mode (no hooks, ARCHON.md, MCP auto-start) |
| `--init` / `--init-only` | Run init hooks then continue / exit |
| `--disable-slash-commands` | Disable slash command parsing |

### Remote / web

| Flag | Purpose |
|---|---|
| `--remote-url <URL>` | Remote URL for `/session` QR display |

### MCP & directories

| Flag | Purpose |
|---|---|
| `--mcp-config <FILES>` | MCP config files (repeatable) |
| `--strict-mcp-config` | Use only `--mcp-config` files (skip discovery) |
| `--add-dir <PATHS>` | Additional working directories for file access |

### Tool restriction

| Flag | Purpose |
|---|---|
| `--tools <LIST>` | Whitelist available tools |
| `--allowed-tools <PATTERNS>` | Tools that skip permission checks |
| `--disallowed-tools <PATTERNS>` | Tools removed from model context |

### Background sessions

| Flag | Purpose |
|---|---|
| `--bg [QUERY]` / `--bg-name <NAME>` | Spawn background session |
| `--ps` | List background sessions |
| `--attach <ID>` | Attach to background session |
| `--kill <ID>` | Terminate background session |
| `--logs <ID>` | Tail background session logs |

### Configuration

| Flag | Purpose |
|---|---|
| `--settings <PATH>` | Additional TOML settings overlay |
| `--setting-sources <LAYERS>` | Comma-separated config layers (`user,project,local`) |

### Observability

| Flag | Purpose |
|---|---|
| `--metrics-port <PORT>` | Prometheus `/metrics` exporter port (0 disables) |
| `--verbose` | info → debug |
| `--debug [CATEGORIES]` | Debug filter (e.g. `--debug api,llm,memory`) |
| `--debug-file <PATH>` | Write debug logs to file |

### Themes & info

| Flag | Purpose |
|---|---|
| `--list-themes` | List TUI themes |
| `--list-output-styles` | List output styles |
| `--list-tools` | List built-in tools |
| `--version` / `-V` | Print version |
| `--help` / `-h` | Print help |

## See also

- [Environment variables](env-vars.md)
- [Configuration](config.md)
- [Slash commands](slash-commands.md)
