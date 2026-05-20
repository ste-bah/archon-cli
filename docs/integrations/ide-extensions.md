# IDE extensions

archon-cli runs inside IDEs via the `ide-stdio` subcommand, which exposes a JSON-RPC protocol over stdin/stdout.

## Protocol

`archon ide-stdio` reads JSON-RPC 2.0 requests on stdin and writes responses to stdout. Notifications stream alongside (no `id` field).

```jsonc
// Request: initialize a session
{ "jsonrpc": "2.0", "id": 1, "method": "archon/initialize", "params": { "clientInfo": { "name": "vscode-archon", "version": "0.1.0" }, "capabilities": { "inlineCompletion": false, "toolExecution": true, "diff": true, "terminal": true } } }

// Response: initialized server session
{ "jsonrpc": "2.0", "id": 1, "result": { "sessionId": "...", "serverVersion": "1.3.2", "capabilities": { "inlineCompletion": false, "toolExecution": false, "diff": false, "terminal": false } } }

// Request: queue a prompt for that session
{ "jsonrpc": "2.0", "id": 2, "method": "archon/prompt", "params": { "sessionId": "...", "text": "hello", "contextFiles": [] } }

// Response: prompt accepted
{ "jsonrpc": "2.0", "id": 2, "result": { "queued": true } }

// Request: fetch protocol status
{ "jsonrpc": "2.0", "id": 3, "method": "archon/status", "params": { "sessionId": "..." } }

// Response: status snapshot
{ "jsonrpc": "2.0", "id": 3, "result": { "model": "claude-sonnet-4-6", "inputTokens": 0, "outputTokens": 0, "cost": 0.0 } }

// Notification: streamed assistant text
{ "jsonrpc": "2.0", "method": "archon/textDelta", "params": { "sessionId": "...", "text": "hi" } }
```

Implemented methods are `archon/initialize`, `archon/prompt`, `archon/cancel`,
`archon/toolResult`, `archon/status`, and `archon/config`. The handler surface
lives in `crates/archon-sdk/src/ide/handler.rs`.

The event bridge emits notifications such as `archon/textDelta`,
`archon/thinkingDelta`, `archon/toolCall`, `archon/permissionRequest`, and
`archon/turnComplete`.

## VS Code

The extension lives in this repo at
[`extensions/vscode/`](../../extensions/vscode/). It wraps
`archon ide-stdio` and surfaces the agent as a VS Code chat panel with
permission prompts, tool-call traces, and slash-command access.

**Marketplace status:** not yet published. Install from source per
[`extensions/vscode/README.md`](../../extensions/vscode/README.md).

The extension treats archon as a chat panel backend. It surfaces:
- Inline assistant messages with tool call traces
- Permission prompts as VS Code modal dialogs
- Session management via the activity bar
- Slash commands as VS Code commands

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
