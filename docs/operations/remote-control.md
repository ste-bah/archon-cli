# Remote control

archon-cli can be controlled remotely via WebSocket, SSH, web UI, or headless JSON-lines mode.

## WebSocket server

Start the server:
```bash
archon serve --port 8420 --token-path ~/.config/archon/serve-token
```

Connect from another machine:
```bash
archon remote ws ws://server.example.com:8420 --token "$(cat ~/.config/archon/serve-token)"
```

The remote client streams bidirectionally — typing in the client appears in the server's TUI session, and server output streams back.

Configure:
```toml
[ws_remote]
port = 8420
# tls_cert = "/path/to/cert.pem"
# tls_key = "/path/to/key.pem"
```

For TLS-enabled remote control, use `wss://`.

## SSH

For machines reachable via SSH, archon-cli can launch itself remotely:

```bash
archon remote ssh user@server.example.com
```

This SSHes to the target, runs `archon ide-stdio` over the SSH transport, and bridges to your local TUI. SSH agent forwarding can be enabled:

```toml
[remote.ssh]
agent_forwarding = false   # set true to forward $SSH_AUTH_SOCK
```

## Web UI

Launch a browser-based UI:
```bash
archon web --port 8421 --bind-address 127.0.0.1
```

Opens `http://localhost:8421` (with `--no-open` to skip auto-launch). The web UI surfaces a chat interface, session manager, agent picker, and tool inspector backed by the same JSON-RPC protocol as `archon ide-stdio`.

Configure:
```toml
[web]
port = 8421
bind_address = "127.0.0.1"
open_browser = true
```

## Headless mode

For backend integration, automation, or programmatic use:
```bash
archon --headless --session-id my-pipeline-001
```

JSON-lines on stdin/stdout. No TUI, no interactive prompts. The configured permission mode is used autonomously.

```jsonc
// Send a message
{"jsonrpc":"2.0","id":1,"method":"session.send","params":{"text":"task description"}}

// Receive streaming deltas
{"jsonrpc":"2.0","method":"session.delta","params":{"text":"...","session_id":"..."}}

// Receive final response
{"jsonrpc":"2.0","id":1,"result":{"text":"...","tokens":...}}
```

## Print mode (one-shot)

For non-interactive single-query use:
```bash
archon -p "summarize Cargo.toml" --output-format json
```

Or pipe stdin:
```bash
echo "what does this do?" | archon -p --output-format text
```

Useful for shell scripts, cron jobs, and CI pipelines.

## Session sharing via QR code

In an interactive TUI session, run:
```
/session
```

This shows a QR code + URL pointing at your `archon serve` endpoint. Scan the QR with another device to connect. Useful for:
- Pair programming (two devices, one session)
- Mobile companion (phone connects to laptop's running session)
- Demo / show-and-tell

Configure the displayed URL via `--remote-url` or `[ws_remote]` settings.

## Security

- **Tokens:** `archon serve` requires a bearer token. Generate strong tokens; never commit them.
- **TLS:** Use `wss://` for any internet-exposed endpoint. Provide `tls_cert` and `tls_key` in config.
- **Bind address:** default `127.0.0.1` for web UI, `0.0.0.0` for `archon serve`. Restrict to `127.0.0.1` if not exposing remotely.
- **Permissions:** the remote session inherits the server's permission mode. Lock down with `--permission-mode plan` for read-only remote access.

## See also

- [IDE extensions](../integrations/ide-extensions.md) — VS Code / JetBrains use `ide-stdio`
- [CLI flags](../reference/cli-flags.md) — `serve`, `remote`, `web`, `--headless`
- [Configuration](../reference/config.md) — `[ws_remote]`, `[web]`, `[remote.ssh]`
