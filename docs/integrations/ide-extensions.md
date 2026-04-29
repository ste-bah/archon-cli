# IDE extensions

archon-cli runs inside IDEs via the `ide-stdio` subcommand, which exposes a JSON-RPC protocol over stdin/stdout.

## Protocol

`archon ide-stdio` reads JSON-RPC 2.0 requests on stdin and writes responses to stdout. Notifications stream alongside (no `id` field).

```jsonc
// Request: send a user message
{ "jsonrpc": "2.0", "id": 1, "method": "session.send", "params": { "text": "hello" } }

// Response: assistant text
{ "jsonrpc": "2.0", "id": 1, "result": { "text": "hi", "tokens": 10 } }

// Notification: streaming text delta
{ "jsonrpc": "2.0", "method": "session.delta", "params": { "text": "hi", "session_id": "..." } }
```

Methods include `session.send`, `session.fork`, `session.resume`, `tools.list`, `agents.list`, `permissions.set`, `metrics.snapshot`. The full method surface lives in `crates/archon-sdk/src/ide.rs`.

## VS Code

A VS Code extension (separate repo) wraps `archon ide-stdio`:

```bash
# In VS Code terminal
archon ide-stdio
```

The extension treats archon as a chat panel backend. It surfaces:
- Inline assistant messages with tool call traces
- Permission prompts as VS Code modal dialogs
- Session management via the activity bar
- Slash commands as VS Code commands

Install from the VS Code marketplace (when published) or build from source at the extension's repo.

## JetBrains

A JetBrains plugin uses the same `archon ide-stdio` protocol. Tool calls, permission prompts, and session state surface as native JetBrains tool windows.

Install from the JetBrains plugin repository (when published).

## Headless mode

For backend integration without an IDE, use `--headless`:

```bash
archon --headless --session-id my-session
```

Same JSON-RPC protocol as `ide-stdio` but assumes no TUI rendering and no interactive permission prompts (uses the configured permission mode autonomously).

## See also

- [Remote control](../operations/remote-control.md) — WebSocket server, web UI
- [CLI flags](../reference/cli-flags.md) — `--headless` and `--session-id`
