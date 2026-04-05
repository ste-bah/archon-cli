/**
 * @archon/sdk — TypeScript client library for Archon IDE extensions.
 *
 * Implements the JSON-RPC 2.0 protocol over WebSocket, with a separate
 * stdio transport for IDE-spawned Archon processes.
 *
 * JSON-RPC framing:
 *   Request:      {"jsonrpc":"2.0","id":N,"method":"archon/X","params":{...}}
 *   Response:     {"jsonrpc":"2.0","id":N,"result":{...}}
 *   Error resp:   {"jsonrpc":"2.0","id":N,"error":{"code":-32600,"message":"..."}}
 *   Notification: {"jsonrpc":"2.0","method":"archon/X","params":{...}}  (no id)
 */

// ── JSON-RPC 2.0 core types ────────────────────────────────────────────────

export interface JRpcRequest<T = unknown> {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params: T;
}

export interface JRpcResponse<T = unknown> {
  jsonrpc: "2.0";
  id: number;
  result?: T;
  error?: JRpcError;
}

export interface JRpcNotification<T = unknown> {
  jsonrpc: "2.0";
  method: string;
  params: T;
}

export interface JRpcError {
  code: number;
  message: string;
  data?: unknown;
}

/** Standard JSON-RPC 2.0 error codes. */
export const JRpcErrorCode = {
  PARSE_ERROR: -32700,
  INVALID_REQUEST: -32600,
  METHOD_NOT_FOUND: -32601,
  INVALID_PARAMS: -32602,
  INTERNAL_ERROR: -32603,
} as const;

// ── IDE-specific parameter and result types ────────────────────────────────

export interface IdeCapabilities {
  inlineCompletion: boolean;
  toolExecution: boolean;
  diff: boolean;
  terminal: boolean;
}

export interface IdeClientInfo {
  name: string;
  version: string;
}

export interface IdeInitializeParams {
  clientInfo: IdeClientInfo;
  capabilities: IdeCapabilities;
}

export interface IdeInitializeResult {
  sessionId: string;
  serverVersion: string;
  capabilities: IdeCapabilities;
}

export interface IdePromptParams {
  sessionId: string;
  text: string;
  contextFiles?: string[];
}

export interface IdeCancelParams {
  sessionId: string;
}

export interface IdeToolResultParams {
  sessionId: string;
  toolUseId: string;
  result: string;
  isError: boolean;
}

export interface IdeStatusParams {
  sessionId: string;
}

export interface IdeStatusResult {
  model: string;
  inputTokens: number;
  outputTokens: number;
  cost: number;
}

export interface IdeConfigParams {
  key?: string;
  value?: unknown;
}

export interface IdeConfigResult {
  value?: unknown;
  ok?: boolean;
}

// ── Server → client notification payload types ─────────────────────────────

export interface IdeTextDelta {
  sessionId: string;
  text: string;
}

export interface IdeThinkingDelta {
  sessionId: string;
  thinking: string;
}

export interface IdeToolCall {
  sessionId: string;
  toolUseId: string;
  name: string;
  input: unknown;
}

export interface IdePermissionRequest {
  sessionId: string;
  action: string;
  description: string;
}

export interface IdeTurnComplete {
  sessionId: string;
  inputTokens: number;
  outputTokens: number;
  cost: number;
}

export interface IdeErrorNotification {
  sessionId?: string;
  message: string;
  code: number;
}

// ── Internal pending-request tracker ──────────────────────────────────────

interface PendingRequest {
  resolve: (value: JRpcResponse) => void;
  reject: (reason: Error) => void;
}

// ── ArchonClient (WebSocket transport) ────────────────────────────────────

/**
 * WebSocket-based Archon client for IDE extensions.
 *
 * Usage:
 * ```typescript
 * const client = new ArchonClient();
 * await client.connect("ws://localhost:7474/ws/ide");
 * const result = await client.initialize({ name: "my-ext", version: "1.0" }, caps);
 * client.onTextDelta((sessionId, text) => process.stdout.write(text));
 * await client.sendPrompt(result.sessionId, "Hello!");
 * ```
 */
export class ArchonClient {
  private ws: WebSocket | null = null;
  private pending: Map<number, PendingRequest> = new Map();
  private nextId = 1;

  // Notification handlers
  private textDeltaHandlers: Array<(sessionId: string, text: string) => void> = [];
  private thinkingDeltaHandlers: Array<(sessionId: string, thinking: string) => void> = [];
  private toolCallHandlers: Array<(sessionId: string, toolUseId: string, name: string, input: unknown) => void> = [];
  private permissionRequestHandlers: Array<(sessionId: string, action: string, description: string) => void> = [];
  private turnCompleteHandlers: Array<(sessionId: string, inputTokens: number, outputTokens: number, cost: number) => void> = [];
  private errorHandlers: Array<(sessionId: string | undefined, message: string, code: number) => void> = [];

  /**
   * Open a WebSocket connection to the Archon server.
   *
   * @param url - WebSocket URL, e.g. `"ws://localhost:7474/ws/ide"`
   */
  connect(url: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      this.ws = ws;

      ws.onopen = () => resolve();

      ws.onerror = (event) => {
        reject(new Error(`WebSocket error: ${String(event)}`));
      };

      ws.onclose = () => {
        // Reject all pending requests on close
        for (const [, pending] of this.pending) {
          pending.reject(new Error("WebSocket closed"));
        }
        this.pending.clear();
      };

      ws.onmessage = (event: MessageEvent<string>) => {
        this.handleMessage(event.data);
      };
    });
  }

  /**
   * Send `archon/initialize` and return the server result.
   *
   * @param clientInfo - Name and version of the IDE extension
   * @param capabilities - Capabilities the client supports
   */
  async initialize(
    clientInfo: IdeClientInfo,
    capabilities: IdeCapabilities
  ): Promise<IdeInitializeResult> {
    const params: IdeInitializeParams = { clientInfo, capabilities };
    const resp = await this.request<IdeInitializeResult>("archon/initialize", params);
    return resp;
  }

  /**
   * Send `archon/prompt` — queues a user prompt in the active session.
   *
   * @param sessionId - Session ID from `initialize`
   * @param text - Prompt text
   * @param contextFiles - Optional list of file paths to include as context
   */
  async sendPrompt(sessionId: string, text: string, contextFiles?: string[]): Promise<void> {
    const params: IdePromptParams = { sessionId, text, contextFiles };
    await this.request<{ queued: boolean }>("archon/prompt", params);
  }

  /**
   * Send `archon/cancel` — request cancellation of the current turn.
   *
   * @param sessionId - Session ID to cancel
   */
  async cancel(sessionId: string): Promise<boolean> {
    const params: IdeCancelParams = { sessionId };
    const resp = await this.request<{ cancelled: boolean }>("archon/cancel", params);
    return resp.cancelled;
  }

  /**
   * Send `archon/toolResult` — return a tool execution result to the agent.
   */
  async toolResult(
    sessionId: string,
    toolUseId: string,
    result: string,
    isError = false
  ): Promise<void> {
    const params: IdeToolResultParams = { sessionId, toolUseId, result, isError };
    await this.request<{ ok: boolean }>("archon/toolResult", params);
  }

  /**
   * Send `archon/status` — query token usage and cost for a session.
   */
  async status(sessionId: string): Promise<IdeStatusResult> {
    const params: IdeStatusParams = { sessionId };
    return this.request<IdeStatusResult>("archon/status", params);
  }

  /**
   * Send `archon/config` — read or write a configuration value.
   *
   * @param key - Configuration key (read if value is omitted)
   * @param value - If provided, writes this value
   */
  async config(key?: string, value?: unknown): Promise<IdeConfigResult> {
    const params: IdeConfigParams = { key, value };
    return this.request<IdeConfigResult>("archon/config", params);
  }

  // ── Notification subscriptions ───────────────────────────────────────────

  /** Register a handler for `archon/textDelta` notifications. */
  onTextDelta(handler: (sessionId: string, text: string) => void): void {
    this.textDeltaHandlers.push(handler);
  }

  /** Register a handler for `archon/thinkingDelta` notifications. */
  onThinkingDelta(handler: (sessionId: string, thinking: string) => void): void {
    this.thinkingDeltaHandlers.push(handler);
  }

  /** Register a handler for `archon/toolCall` notifications. */
  onToolCall(
    handler: (sessionId: string, toolUseId: string, name: string, input: unknown) => void
  ): void {
    this.toolCallHandlers.push(handler);
  }

  /** Register a handler for `archon/permissionRequest` notifications. */
  onPermissionRequest(
    handler: (sessionId: string, action: string, description: string) => void
  ): void {
    this.permissionRequestHandlers.push(handler);
  }

  /** Register a handler for `archon/turnComplete` notifications. */
  onTurnComplete(
    handler: (sessionId: string, inputTokens: number, outputTokens: number, cost: number) => void
  ): void {
    this.turnCompleteHandlers.push(handler);
  }

  /** Register a handler for `archon/error` notifications. */
  onError(
    handler: (sessionId: string | undefined, message: string, code: number) => void
  ): void {
    this.errorHandlers.push(handler);
  }

  /** Close the WebSocket connection. */
  disconnect(): void {
    this.ws?.close();
    this.ws = null;
  }

  // ── Internal ─────────────────────────────────────────────────────────────

  private request<T>(method: string, params: unknown): Promise<T> {
    return new Promise((resolve, reject) => {
      const ws = this.ws;
      if (!ws || ws.readyState !== WebSocket.OPEN) {
        reject(new Error("WebSocket is not connected"));
        return;
      }

      const id = this.nextId++;
      const request: JRpcRequest = { jsonrpc: "2.0", id, method, params };

      this.pending.set(id, {
        resolve: (resp: JRpcResponse) => {
          if (resp.error) {
            reject(
              new Error(`JSON-RPC error ${resp.error.code}: ${resp.error.message}`)
            );
          } else {
            resolve(resp.result as T);
          }
        },
        reject,
      });

      ws.send(JSON.stringify(request));
    });
  }

  private handleMessage(data: string): void {
    let msg: unknown;
    try {
      msg = JSON.parse(data) as unknown;
    } catch {
      return;
    }

    if (typeof msg !== "object" || msg === null) return;
    const obj = msg as Record<string, unknown>;

    // Determine if this is a response (has `id` and `result`/`error`) or a notification.
    if ("id" in obj && typeof obj["id"] === "number") {
      const resp = msg as JRpcResponse;
      const pending = this.pending.get(resp.id);
      if (pending) {
        this.pending.delete(resp.id);
        pending.resolve(resp);
      }
      return;
    }

    // Notification: no `id` field.
    if ("method" in obj && typeof obj["method"] === "string") {
      const notif = msg as JRpcNotification<Record<string, unknown>>;
      this.dispatchNotification(notif.method, notif.params ?? {});
    }
  }

  private dispatchNotification(method: string, params: Record<string, unknown>): void {
    switch (method) {
      case "archon/textDelta": {
        const p = params as unknown as IdeTextDelta;
        for (const h of this.textDeltaHandlers) h(p.sessionId, p.text);
        break;
      }
      case "archon/thinkingDelta": {
        const p = params as unknown as IdeThinkingDelta;
        for (const h of this.thinkingDeltaHandlers) h(p.sessionId, p.thinking);
        break;
      }
      case "archon/toolCall": {
        const p = params as unknown as IdeToolCall;
        for (const h of this.toolCallHandlers) h(p.sessionId, p.toolUseId, p.name, p.input);
        break;
      }
      case "archon/permissionRequest": {
        const p = params as unknown as IdePermissionRequest;
        for (const h of this.permissionRequestHandlers) h(p.sessionId, p.action, p.description);
        break;
      }
      case "archon/turnComplete": {
        const p = params as unknown as IdeTurnComplete;
        for (const h of this.turnCompleteHandlers)
          h(p.sessionId, p.inputTokens, p.outputTokens, p.cost);
        break;
      }
      case "archon/error": {
        const p = params as unknown as IdeErrorNotification;
        for (const h of this.errorHandlers) h(p.sessionId, p.message, p.code);
        break;
      }
      default:
        break;
    }
  }
}

// ── ArchonStdioClient (stdio / JSON-lines transport) ──────────────────────

/**
 * Stdio JSON-lines transport for IDE extensions that spawn an Archon process.
 *
 * Reads newline-delimited JSON-RPC messages from `process.stdin` and writes
 * responses to `process.stdout`. Intended for use inside the spawned Archon
 * process, not in the IDE extension itself.
 *
 * Usage (inside the archon process):
 * ```typescript
 * const client = new ArchonStdioClient();
 * client.onTextDelta((sid, text) => { ... });
 * client.start();
 * ```
 */
export class ArchonStdioClient {
  private pending: Map<number, PendingRequest> = new Map();
  private nextId = 1;
  private buffer = "";

  private textDeltaHandlers: Array<(sessionId: string, text: string) => void> = [];
  private turnCompleteHandlers: Array<(sessionId: string, inputTokens: number, outputTokens: number, cost: number) => void> = [];

  /** Register a handler for `archon/textDelta` notifications. */
  onTextDelta(handler: (sessionId: string, text: string) => void): void {
    this.textDeltaHandlers.push(handler);
  }

  /** Register a handler for `archon/turnComplete` notifications. */
  onTurnComplete(
    handler: (sessionId: string, inputTokens: number, outputTokens: number, cost: number) => void
  ): void {
    this.turnCompleteHandlers.push(handler);
  }

  /**
   * Start reading from `process.stdin`. Each complete line is parsed as a
   * JSON-RPC message and dispatched.
   */
  start(): void {
    if (typeof process === "undefined") return;

    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk: string) => {
      this.buffer += chunk;
      const lines = this.buffer.split("\n");
      this.buffer = lines.pop() ?? "";
      for (const line of lines) {
        const trimmed = line.trim();
        if (trimmed.length > 0) {
          this.handleLine(trimmed);
        }
      }
    });
  }

  /** Send a JSON-RPC request via stdout and return a promise for the result. */
  private request<T>(method: string, params: unknown): Promise<T> {
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      const req: JRpcRequest = { jsonrpc: "2.0", id, method, params };
      this.pending.set(id, {
        resolve: (resp: JRpcResponse) => {
          if (resp.error) {
            reject(new Error(`JSON-RPC error ${resp.error.code}: ${resp.error.message}`));
          } else {
            resolve(resp.result as T);
          }
        },
        reject,
      });
      process.stdout.write(JSON.stringify(req) + "\n");
    });
  }

  /** Send `archon/initialize` via stdout. */
  initialize(clientInfo: IdeClientInfo, capabilities: IdeCapabilities): Promise<IdeInitializeResult> {
    return this.request<IdeInitializeResult>("archon/initialize", { clientInfo, capabilities });
  }

  /** Send `archon/prompt` via stdout. */
  sendPrompt(sessionId: string, text: string): Promise<void> {
    return this.request<{ queued: boolean }>("archon/prompt", { sessionId, text }).then(() => undefined);
  }

  private handleLine(line: string): void {
    let msg: unknown;
    try {
      msg = JSON.parse(line) as unknown;
    } catch {
      return;
    }
    if (typeof msg !== "object" || msg === null) return;
    const obj = msg as Record<string, unknown>;

    if ("id" in obj && typeof obj["id"] === "number") {
      const resp = msg as JRpcResponse;
      const pending = this.pending.get(resp.id);
      if (pending) {
        this.pending.delete(resp.id);
        pending.resolve(resp);
      }
      return;
    }

    if ("method" in obj && typeof obj["method"] === "string") {
      const notif = msg as JRpcNotification<Record<string, unknown>>;
      switch (notif.method) {
        case "archon/textDelta": {
          const p = notif.params as unknown as IdeTextDelta;
          for (const h of this.textDeltaHandlers) h(p.sessionId, p.text);
          break;
        }
        case "archon/turnComplete": {
          const p = notif.params as unknown as IdeTurnComplete;
          for (const h of this.turnCompleteHandlers)
            h(p.sessionId, p.inputTokens, p.outputTokens, p.cost);
          break;
        }
        default:
          break;
      }
    }
  }
}
