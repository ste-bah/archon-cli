# Domain Context: IDE Extensions and JSON-RPC Protocol

## Background
IDE extensions communicate with language tools via structured protocols. This project uses
JSON-RPC 2.0 (the same base protocol as LSP/MCP) over two transports:
1. **WebSocket**: IDE connects to a running `archon serve` process
2. **Stdio**: IDE spawns `archon --ide-stdio`, reads/writes JSON-RPC on stdin/stdout

## JSON-RPC 2.0 Basics
```json
// Request
{"jsonrpc":"2.0","id":1,"method":"archon/prompt","params":{"text":"explain this code"}}
// Response
{"jsonrpc":"2.0","id":1,"result":{"sessionId":"abc"}}
// Notification (no id, no response expected)
{"jsonrpc":"2.0","method":"archon/textDelta","params":{"text":"Here is an explanation..."}}
// Error
{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid Request","data":{}}}
```

## VS Code Extension Key Concepts
- `vscode.commands.registerCommand()` — register commands
- `vscode.window.createWebviewPanel()` — create chat panel
- `vscode.languages.registerCodeActionsProvider()` — code actions
- `vscode.window.createTextEditorDecorationType()` — inline hints
- `vscode.workspace.applyEdit()` — apply file changes
- Message format between extension and webview: `panel.webview.postMessage()` / `onDidReceiveMessage`
- Extension activates via `activationEvents` in `package.json`

## JetBrains Plugin Key Concepts
- `ToolWindowFactory` — creates the chat tool window
- `IntentionAction` — Alt+Enter actions
- `AnAction` — toolbar/menu actions
- `WriteCommandAction.runWriteCommandAction()` — file modifications with undo support
- `ApplicationManager.getApplication().executeOnPooledThread()` — background I/O
- `SwingUtilities.invokeLater()` / `UIUtil.invokeLaterIfNeeded()` — UI updates on EDT
- Plugin descriptor: `plugin.xml` with `<idea-plugin>` root
- Build: Gradle + `org.jetbrains.intellij` plugin

## Archon JSON-RPC Methods
```typescript
// Client → Server (requests)
archon/initialize: { clientInfo: {name, version}, capabilities: {inlineCompletion, toolExecution, diff, terminal} }
archon/prompt: { sessionId: string, text: string, contextFiles?: string[] }
archon/cancel: { sessionId: string }
archon/toolResult: { sessionId: string, toolUseId: string, result: string, isError: boolean }
archon/status: { sessionId: string }
archon/config: { key?: string, value?: unknown }  // get or set

// Server → Client (notifications)
archon/textDelta: { sessionId: string, text: string }
archon/thinkingDelta: { sessionId: string, thinking: string }
archon/toolCall: { sessionId: string, toolUseId: string, name: string, input: unknown }
archon/permissionRequest: { sessionId: string, action: string, description: string }
archon/turnComplete: { sessionId: string, inputTokens: number, outputTokens: number, cost: number }
archon/error: { sessionId?: string, message: string, code: number }
```

## TypeScript SDK Pattern
```typescript
class ArchonClient {
  private ws: WebSocket;
  private pending: Map<number, {resolve, reject}> = new Map();
  private nextId = 1;

  async initialize(caps: Capabilities): Promise<InitializeResult> {
    return this.request('archon/initialize', { clientInfo: {...}, capabilities: caps });
  }

  private request<T>(method: string, params: unknown): Promise<T> {
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      this.pending.set(id, {resolve, reject});
      this.ws.send(JSON.stringify({jsonrpc:'2.0', id, method, params}));
    });
  }
}
```

## Kotlin SDK Pattern
```kotlin
class ArchonClient(private val url: String) {
    private var ws: WebSocketSession? = null
    private val pending = ConcurrentHashMap<Int, CompletableDeferred<JsonObject>>()
    private var nextId = AtomicInteger(1)

    suspend fun initialize(caps: Capabilities): InitializeResult = request("archon/initialize", caps)

    private suspend fun request(method: String, params: Any): JsonObject {
        val id = nextId.getAndIncrement()
        val deferred = CompletableDeferred<JsonObject>()
        pending[id] = deferred
        ws?.send(buildJsonObject {
            put("jsonrpc", "2.0"); put("id", id); put("method", method)
        }.toString())
        return deferred.await()
    }
}
```
