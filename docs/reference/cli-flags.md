# CLI flags

Run `archon --help` for the live, authoritative listing. Every flag below is verified against `src/cli_args.rs`.

For which shell command families are mirrored into slash/TUI flows, see the generated [command surface matrix](../generated/command-surface-matrix.md).

Provider selection is shared across shell and TUI agentic surfaces. Use
`archon auth login --provider anthropic` for Anthropic OAuth/API-key/proxy
workflows, or `archon auth login --provider openai-codex` plus
`[llm].provider = "openai-codex"` to route the TUI, `/btw`, team runs,
subagents, coding/research pipelines, and gametheory through Codex.

## Subcommands

| Subcommand | Synopsis |
|---|---|
| `archon` | Start interactive TUI (default) |
| `archon login` | Legacy shortcut for Anthropic OAuth PKCE login |
| `archon logout` | Legacy shortcut for Anthropic logout |
| `archon auth login --provider anthropic\|openai-codex` | Authenticate a provider |
| `archon auth status` | Show redacted Anthropic/Codex auth status |
| `archon chat --provider <ID> <PROMPT>` | Single-turn chat through a selected provider |
| `archon providers [list\|capabilities\|status [--live]\|doctor [--live]]` | List provider registry entries, surface capability support, show redacted runtime status, local auth diagnostics, or opt-in endpoint reachability checks |
| `archon serve [--port PORT] [--token-path PATH]` | Start WebSocket server for remote agent access |
| `archon remote ws <URL> [--token TOKEN]` | Connect to remote agent via WebSocket |
| `archon remote ssh <TARGET>` | Connect to remote agent via SSH |
| `archon web [--port PORT] [--bind-address ADDR] [--no-open]` | Start browser-based web workbench |
| `archon team run --team NAME <GOAL>` | Execute a multi-agent team on a goal using the configured provider |
| `archon team list` | List configured teams |
| `archon plugin list` | List discovered plugins |
| `archon plugin info <NAME>` | Show plugin details |
| `archon ide-stdio` | Run in IDE stdio mode (JSON-RPC over stdin/stdout) |
| `archon pipeline code <TASK> [--dry-run]` | Run the coding pipeline on a task using the configured provider |
| `archon pipeline research <TOPIC> [--dry-run]` | Run the research pipeline on a topic using the configured provider |
| `archon pipeline status <SESSION_ID>` | Show pipeline session status |
| `archon pipeline resume <SESSION_ID> [--force-quality-gate]` | Resume an interrupted pipeline session; the force flag audits and continues past critical quality-gate failure only |
| `archon pipeline rewind <SESSION_ID> --to-agent <KEY>` | Quarantine completed audited agent records from `<KEY>` onward so the next resume re-runs them |
| `archon pipeline rewind <SESSION_ID> --to-ordinal <N>` | Rewind to an accepted agent ordinal when the agent key is unavailable |
| `archon pipeline rewind <SESSION_ID> --keep-agents <N>` | Low-level rewind form that keeps exactly `N` completed agent records |
| `archon pipeline list` | List all pipeline sessions |
| `archon pipeline abort <SESSION_ID>` | Abort a running pipeline session |
| `archon pipeline verify <SESSION_ID> [--write-report]` | Verify an audited built-in pipeline bundle and optionally write `verification/report.json` |
| `archon pipeline inspect <SESSION_ID>` | Inspect an audited built-in pipeline bundle manifest, state, and agent records |
| `archon pipeline export-traces <SESSION_ID> [--format jsonl] [--out PATH] [--include-unverified]` | Export per-attempt audited pipeline traces as JSONL |
| `archon pipeline run <FILE> [--format FMT] [--detach]` | Run declarative pipeline from spec file |
| `archon pipeline cancel <ID>` | Cancel a running declarative pipeline |
| `archon workflow plan <TASK>` | Create a provider-neutral dynamic workflow spec without executing it |
| `archon workflow run <TASK>` | Create and execute a durable dynamic workflow under `.archon/workflows/<run-id>` |
| `archon workflow status <RUN_ID>` | Show dynamic workflow status and stage counts |
| `archon workflow resume <RUN_ID>` | Resume a dynamic workflow from durable state |
| `archon workflow restart-agent <RUN_ID> <STAGE_ID>` | Rewind one workflow stage/agent before resume |
| `archon workflow save <RUN_ID> <NAME>` | Save a sanitized reusable workflow template |
| `archon workflow list` | List dynamic workflow runs |
| `archon self retrospective <SESSION_ID> [--analyzer hybrid\|heuristic\|llm]` | Extract evidence-backed lessons from a session activity log |
| `archon self trust status` | Show domain-scoped self-calibration trust scores |
| `archon self plans inspect <SESSION_ID>` | Compare a stored session plan with recorded step outcomes |
| `archon world status` | Show local world-model corpus, cold-start, and backend status |
| `archon world ingest <SESSION_ID>` | Ingest one session's activity, plan, memory, retrospective, transcript, provider, pipeline, and agent-output artifacts into the corpus |
| `archon world ingest --backfill` | Backfill available session, pipeline, memory, retrospective, provider, plugin-artifact, and agent-output traces into the corpus |
| `archon world predict-next --session-id <ID> --action-ref <REF> --summary <TEXT>` | Request a fail-open next-state advisory |
| `archon world score-actions --task <TEXT> --actions <PATH>` | Rank candidate actions with similarity-based counterfactual scoring |
| `archon world explain <PREDICTION_ID>` | Inspect a persisted prediction and any recorded outcome/surprise |
| `archon world record-outcome <PREDICTION_ID> --actual-summary <TEXT>` | Attach a redacted actual next-state summary and compute latent surprise |
| `archon world train [--candidate] [--max-runtime-ms MS]` | Train a local candidate manifest from the stored world-model corpus |
| `archon world train-jepa [--candidate] [--max-runtime-ms MS]` | Train a JEPA-inspired representation candidate from the stored world-model corpus |
| `archon world trainer-tick [--last-activity-age-ms MS] [--last-training-age-ms MS] [--battery-percent N] [--unplugged]` | Run one idle-aware dynamic trainer tick |
| `archon world eval [CANDIDATE_ID]` | Evaluate a candidate manifest against mandatory promotion gates |
| `archon world eval-jepa <CANDIDATE_ID> [--full] [--background] [--resume RUN_ID] [--backend cpu\|metal\|cuda] [--no-cache]` | Evaluate a JEPA-inspired candidate. Default mode is quick Tier-0; use `--full` before promotion. `--background`, `--backend`, and `--no-cache` currently emit explicit deferral warnings/errors instead of silently changing execution. |
| `archon world eval-jepa-status <RUN_ID>` | Inspect a persisted JEPA eval run record |
| `archon world eval-jepa-runs [--limit N]` | List recent JEPA eval run records |
| `archon world eval-jepa-cancel <RUN_ID>` | Write a cancellation sentinel for a JEPA eval run |
| `archon world inspect-jepa <CANDIDATE_ID>` | Inspect a JEPA-inspired candidate manifest and prior eval state |
| `archon world compare-representations --baseline fastembed --candidate <CANDIDATE_ID>` | Compare a JEPA representation against an exploratory baseline. Promotion still uses the fixed FastEmbed baseline. |
| `archon world promote <MODEL_ID>` | Promote only a candidate with a passing eval report |
| `archon world promote-jepa <CANDIDATE_ID>` | Promote a JEPA-inspired candidate only after a full promotion-grade eval passes |
| `archon world rollback <MODEL_ID>` | Restore a prior advisory model pointer |
| `archon reasoning status` | Show reasoning-quality store, shadow, critic, and dead-letter status |
| `archon reasoning inspect <SESSION_ID>` | Summarize reasoning-quality events for a session |
| `archon reasoning claims <SESSION_ID>` | List captured claim/evidence rows for a session |
| `archon reasoning patterns` | Show repeated reasoning-failure clusters |
| `archon reasoning backfill [--sessions N] [--emit-world-rows]` | Deterministically extract reasoning-quality rows from stored sessions |
| `archon reasoning fixture-audit` | Audit labeled extractor fixtures for secrets and quality gates |
| `archon reasoning cost status` | Show optional LLM critic budget usage |
| `archon reasoning replay-dead-letter [--bridge NAME]` | Replay recoverable reasoning-quality bridge failures |
| `archon briefing preview [--task TEXT]` | Preview proactive session-start briefing content |
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
| `--json-schema <SCHEMA>` | Validate final assistant output against an inline JSON schema string |
| `--json-schema-path <PATH>` | Validate final assistant output against a JSON schema file |
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
| `--model <MODEL>` | Override default model. In Codex sessions, inherited Claude-shaped defaults are normalized through `[models.openai-codex]`; explicit overrides are preserved. |
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

See [Web workbench](../operations/web-workbench.md) for the browser tab guide,
data sources, and action safety model.

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
