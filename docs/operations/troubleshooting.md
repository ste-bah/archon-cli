# Troubleshooting

Known issues and recovery procedures, organized by symptom.

## Startup

### `(no auth)` shown in TUI

No credentials found in the resolution order. Either:
- Run `archon login` (OAuth)
- `export ANTHROPIC_API_KEY="sk-ant-..."`

See [env-vars resolution order](../reference/env-vars.md#resolution-order-for-credentials).

### Slow startup (~5s on first launch)

First-time CozoDB schema initialization. Subsequent launches are < 500ms. Not a bug.

### `error: package 'archon-cli-workspace' specifies edition 2024`

Rust toolchain too old. `rustup update stable` to get 1.85+.

### Build hangs then `Killed` / `signal: 9` on WSL2

OOM during parallel rustc. Rebuild with `cargo build --release -j1`.

### rustc ICE on `petgraph::graphmap::NeighborsDirected::next`

Stale dep cache corruption (known intermittent issue). Run:
```bash
cargo clean -p petgraph -p archon-pipeline
cargo build --release -j1
```

## Authentication

### `429 Too Many Requests` on every send

Rate limit on your account or shared IP. Wait, or check `/status` for retry timing. Per-account quotas reset hourly.

### OAuth flow fails to complete

The OAuth callback runs on `http://localhost:{random_port}/callback`. If a firewall or VPN blocks the loopback callback, the flow times out. Check:
- Firewall isn't blocking loopback connections
- No other process has bound the random port
- Browser allows redirects to `http://localhost`

Fall back to API key if OAuth keeps timing out:
```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### Pre-existing `oauth.json` ignored

If multiple Claude Code clients write to the same path, file locking prevents corruption but may cause stale-token reads. Run `archon login` to re-authenticate.

## Permissions

### `Blocked. The Agent tool requires elevated permissions`

You're in `default` mode and the Agent tool isn't auto-approved. Switch:
```
/permissions auto
```
Or pass at startup: `archon --permission-mode auto`.

### `Permission denied: Bash:rm /tmp/foo`

Either the rule list explicitly denies it (`always_deny`) or the mode is `plan`/`sandbox`. Check:
```
/denials
/permissions
```

To temporarily allow: `/permissions auto` or add to `always_allow`:
```toml
[permissions]
always_allow = ["Bash:rm /tmp/*"]
```

## Pipelines

### Pipeline agent dispatch panics with `blocking_lock`-style error

Pre-v0.1.13 bug. Upgrade to current release.

### Pipeline session won't resume

Session state requires git working tree consistency. If files were modified mid-pipeline, the recovery layer rejects continuation. Check:
```bash
archon pipeline status <session-id>
git status
```

Either revert the conflicting changes or abort the session and restart.

## MCP

### MCP server not connecting

Check transport, command, and env vars:
```bash
archon --debug mcp
```
The debug log shows the MCP handshake. Common issues:
- `command` not in PATH
- Required env vars (API tokens) missing
- WebSocket endpoint requires TLS but config uses `ws://`

### MCP server connects but tools don't appear

Tool registration happens after the handshake. Check `/mcp` for status. If `tool_count = 0`, the server didn't return tools in its capabilities response — bug in the server, not archon.

### WebSocket reconnect loops

Exponential backoff caps at 30s with a 10-minute retry budget. If the server keeps closing with code 1002/4001/4003, archon halts reconnection (these are "permanent" close codes). Check the server logs.

## Memory

### Memory recall returns empty on first run

No memories yet. Memories accrue from agent activity (AutoCapture). Seed with:
```
/memory store "your seed memory text"
```

Or invoke the `memory_store` tool directly:
```jsonc
{ "memory_type": "Fact", "content": "..." }
```

### Memory garden consolidation never runs

Check throttle:
```toml
[memory.garden]
auto_consolidate = true
min_hours_between_runs = 24
```

If `min_hours_between_runs` is too high, manually trigger:
```
/garden
```

## TUI

### Theme looks wrong colours

Terminal doesn't support truecolor. Run `archon --list-themes` for compatible 16-color themes.

### Slash commands not autocompleting

Registry hasn't picked up newly-dropped agents/skills. Run `/refresh` to re-scan.

### Vim mode keybindings inverted

`Esc` enters Normal mode (not Insert). If your terminal swaps Esc and Caps Lock, fix at the terminal level.

## Logs

Per-session logs at `~/.local/share/archon/logs/<session-id>.log`. Default level is `info`. Bump for diagnostics:

```bash
archon --debug api,llm,memory,mcp,permissions
RUST_LOG=archon=trace archon
```

The log file is human-readable with timestamps, request/response correlation IDs, and tool call summaries.

## Diagnostics

```
/doctor
```

Runs a battery of self-checks: auth status, MCP servers, LSP servers, plugin load, memory graph, learning systems. Output is a triage summary.

## Reporting bugs

```
/bug
```

Opens a GitHub issue prefilled with:
- archon version
- OS / arch / Rust version
- Recent log excerpt (sanitized for credentials)
- Active config (sanitized)

## See also

- [Logs and observability](data-locations.md) — where everything lives
- [Configuration](../reference/config.md) — `[logging]` section
- [GitHub issues](https://github.com/ste-bah/archon-cli/issues) — known issue tracker
