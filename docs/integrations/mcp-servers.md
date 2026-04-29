# MCP servers

Model Context Protocol (MCP) extends archon-cli with external tools and resources from third-party servers. Both stdio and network transports are supported.

## Supported transports

| Transport | Use case |
|---|---|
| `stdio` | Local processes (default) |
| `websocket` (`ws://`, `wss://`) | Remote / network MCP servers |
| `http_streamable` | HTTP streaming (beta) |

## Config schema

`.mcp.json`:

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

Environment variables expand inline (`${VAR}`). Servers with `"disabled": true` are skipped.

## Config loading

archon-cli auto-discovers config from:

1. `~/.config/archon/.mcp.json` — global
2. `<workdir>/.mcp.json` — project-local (overrides global per-server)
3. `--mcp-config FILES...` — repeatable CLI flag
4. `--strict-mcp-config` — use only `--mcp-config` files (skip discovery)

## Reconnection (WebSocket)

Exponential backoff with ±12.5% jitter, capped at 30s. Permanent close codes (1002, 4001, 4003) halt reconnection. A 10-minute retry budget and 60s sleep-gap detection prevent runaway reconnect loops after laptop suspend.

## Slash commands

| Command | Purpose |
|---|---|
| `/mcp` | Show MCP server status |
| `/connect` | List configured MCP servers |
| `/connect <name>` | Show connection hint for a specific server |

## Tools exposed by MCP

When an MCP server connects successfully, archon-cli registers its tools and resources into the agent's tool catalog. The agent sees them like any other tool.

Two built-in tools query MCP server resources directly:

| Tool | Purpose |
|---|---|
| `ListMcpResources` | List resources from connected MCP servers (filter by server) |
| `ReadMcpResource` | Read an MCP resource by URI (text inline, binary base64; truncated at 100KB) |

## Common MCP servers

| Server | Purpose | Install |
|---|---|---|
| `@modelcontextprotocol/server-filesystem` | File access | `npx -y @modelcontextprotocol/server-filesystem` |
| `mcp-server-github` | GitHub API | from your package manager |
| `mcp-server-postgres` | Postgres queries | `npx -y @modelcontextprotocol/server-postgres` |
| `mcp-server-puppeteer` | Browser automation | `npx -y @modelcontextprotocol/server-puppeteer` |
| `memorygraph` | archon-cli's own MCP server | bundled |

See the [MCP server registry](https://modelcontextprotocol.io/registry) for the full ecosystem.

## Debugging MCP

```bash
archon --debug mcp                # debug-level MCP logs
RUST_LOG=archon_mcp=trace archon  # trace transport, JSON-RPC frames
```

In the TUI:
```
/mcp                              # status of all servers
/doctor                           # diagnostics include MCP health
```

## See also

- [Tools](../reference/tools.md) — `ListMcpResources` and `ReadMcpResource`
- [Configuration](../reference/config.md) — global config layering
