/**
 * ConnectionManager — manages the lifecycle of the Archon backend connection.
 *
 * Supports two transports:
 *  - WebSocket  (ConnectionMode.WebSocket) — connects to a running Archon server
 *  - Stdio      (ConnectionMode.Stdio)     — spawns the Archon binary as a child process
 *
 * Callers interact through the public send/disconnect API and register event
 * handlers via `onTextDelta` / `onTurnComplete`.
 */

import type { ChildProcess } from "child_process";
import { ConnectionMode } from "../constants";
import {
  ConnectionState,
  WsConnectionConfig,
  DEFAULT_WS_CONFIG,
  IdeCapabilities,
  PermissionRequest,
  PermissionResolved,
  SessionStatus,
  ToolCall,
  ToolCallComplete,
} from "../types";

/** Token-usage summary delivered when a turn finishes. */
export interface TurnTokens {
  in: number;
  out: number;
}

/**
 * Default capabilities advertised by the VS Code extension during initialize.
 *
 * `toolExecution` is load-bearing rather than cosmetic: the backend reads it
 * as "this client has an allow/deny UI". A client that advertises `false` has
 * every permission request refused immediately, because there would be nobody
 * to answer it. The chat panel renders that UI, so this is `true`.
 */
const DEFAULT_CAPABILITIES: IdeCapabilities = {
  inlineCompletion: false,
  toolExecution: true,
  diff: false,
  terminal: false,
};

/** Pending JSON-RPC request awaiting a response. */
interface PendingRequest {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
}

export class ConnectionManager {
  private _state: ConnectionState = "idle";
  private _ws: WebSocket | null = null;
  private _child: ChildProcess | null = null;
  private _sessionId: string | null = null;
  private _nextId = 1;
  private _pending = new Map<number, PendingRequest>();
  private _stdoutBuffer = "";

  // Public event callbacks
  public onTextDelta: ((text: string) => void) | null = null;
  public onThinkingDelta: ((text: string) => void) | null = null;
  public onToolCall: ((call: ToolCall) => void) | null = null;
  public onToolCallComplete: ((result: ToolCallComplete) => void) | null = null;
  public onPermissionRequest: ((request: PermissionRequest) => void) | null =
    null;
  public onPermissionResolved: ((resolved: PermissionResolved) => void) | null =
    null;
  public onTurnComplete: ((tokens: TurnTokens) => void) | null = null;
  public onError: ((message: string) => void) | null = null;

  /**
   * Diagnostic output from the stdio backend: the child's stderr verbatim,
   * plus one line when the process exits.
   *
   * This is a callback rather than a direct write to a VS Code output channel
   * because this module must stay loadable outside the extension host — it is
   * required directly by the plain-Node test suite, where `vscode` does not
   * resolve, and it is kept free of host-only imports for the same bundling
   * reason as the dynamic `child_process` require below. The activation side
   * owns the channel and points this at it.
   *
   * Left unset, backend diagnostics are discarded: `windowsHide` means there
   * is no console for them to land in, so an unrouted sink is how a crashed
   * backend becomes an unexplained "Archon: error" in the status bar.
   */
  public onBackendLog: ((text: string) => void) | null = null;

  // ── Public API ─────────────────────────────────────────────────────────────

  /** Returns the current connection state. */
  getState(): ConnectionState {
    return this._state;
  }

  /** Returns the active session ID negotiated during initialize, or null. */
  getSessionId(): string | null {
    return this._sessionId;
  }

  /**
   * Establish a WebSocket connection and perform the initialization handshake.
   *
   * @param config - WebSocket endpoint configuration. Defaults to localhost:8420.
   */
  async connect(config: WsConnectionConfig = DEFAULT_WS_CONFIG): Promise<void> {
    this._state = "connecting";

    await new Promise<void>((resolve, reject) => {
      // Use global WebSocket — available in VS Code extension host (Node ≥ 22)
      // and in browser-based webview contexts.
      const ws = new WebSocket(config.url);
      this._ws = ws;

      const headers: Record<string, string> = {};
      if (config.token) {
        headers["Authorization"] = `Bearer ${config.token}`;
      }

      ws.onopen = () => resolve();
      ws.onerror = () => {
        this._state = "error";
        reject(new Error("Archon: WebSocket connection failed"));
      };
      ws.onclose = () => {
        if (this._state === "connected") {
          this._state = "idle";
        }
      };
      ws.onmessage = (event: MessageEvent) => {
        this._handleMessage(String(event.data));
      };
    });

    await this._initialize();
  }

  /**
   * Connect via Archon's stdio transport.
   *
   * The connection manager spawns the Archon binary as a child process and
   * communicates via newline-delimited JSON-RPC on stdin/stdout. After the
   * process is alive, the initialize handshake is performed before returning.
   *
   * @param binaryPath - Path to the `archon` executable.
   * @param mode - Must be ConnectionMode.Stdio (validated at call site).
   * @param workspaceRoot - Project root the agent should work in. Passed both
   *   as the child's cwd and as `--workspace`: the backend resolves project
   *   configuration from its cwd but its working directory from the flag, so
   *   sending only one of the two leaves the two halves disagreeing.
   */
  async connectStdio(
    binaryPath: string,
    _mode: ConnectionMode,
    workspaceRoot?: string
  ): Promise<void> {
    this._state = "connecting";

    // Dynamic require keeps the `child_process` import out of webview bundles.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { spawn } = require("child_process") as typeof import("child_process");

    const args = workspaceRoot
      ? ["ide-stdio", "--workspace", workspaceRoot]
      : ["ide-stdio"];

    await new Promise<void>((resolve, reject) => {
      const child = spawn(binaryPath, args, {
        // stderr is piped rather than inherited: with `windowsHide` there is no
        // console for inherited output to appear in, and the extension host has
        // none either, so inheriting would discard every backend diagnostic.
        stdio: ["pipe", "pipe", "pipe"],
        cwd: workspaceRoot,
        // Windows attaches a console to a console-subsystem child by default.
        // The backend speaks JSON-RPC over stdio and paints nothing, so that
        // console shows up as a blank window that steals foreground from VS
        // Code on every connect and reconnect — and closing it, which is the
        // obvious thing to do with a window that looks hung, kills the
        // extension's backend. Ignored on other platforms.
        windowsHide: true,
      });
      this._child = child;

      child.stdout?.on("data", (chunk: Buffer) => {
        this._stdoutBuffer += chunk.toString("utf8");
        const lines = this._stdoutBuffer.split("\n");
        this._stdoutBuffer = lines.pop() ?? "";
        for (const line of lines) {
          const trimmed = line.trim();
          if (trimmed.length > 0) {
            this._handleMessage(trimmed);
          }
        }
      });

      child.stderr?.on("data", (chunk: Buffer) => {
        this.onBackendLog?.(chunk.toString("utf8"));
      });

      child.on("error", (err: Error) => {
        this._state = "error";
        reject(new Error(`Archon: failed to spawn binary — ${err.message}`));
      });

      child.on("exit", (code: number | null, signal: string | null) => {
        if (this._state === "connected") {
          this._state = "idle";
        }
        // Killing the backend is the one failure with nothing to print: a
        // process terminated from outside writes no stderr on its way out, so
        // without this line the channel is silent about the most likely cause.
        this.onBackendLog?.(
          `\n[archon] backend process exited (${
            signal !== null ? `signal ${signal}` : `code ${code ?? "unknown"}`
          })\n`
        );
        this._rejectAllPending(new Error("Archon: process exited"));
      });

      child.on("spawn", () => resolve());
    });

    await this._initialize();
  }

  /**
   * Send an `archon/prompt` JSON-RPC request to the connected backend.
   *
   * The sessionId from the initialize handshake is used automatically; the
   * `sessionId` argument is retained for backwards compatibility but ignored
   * if a real session ID has been negotiated.
   *
   * @param sessionId - Caller-provided session identifier (legacy).
   * @param text - User prompt text.
   * @param contextFiles - Optional list of workspace-relative file paths.
   */
  async sendPrompt(
    sessionId: string,
    text: string,
    contextFiles?: string[]
  ): Promise<void> {
    const effectiveSessionId = this._sessionId ?? sessionId;
    // Awaited, not fired and forgotten: the backend rejects a prompt while
    // another turn is in flight, and swallowing that leaves the panel spinning
    // on a turn that was never accepted.
    await this._sendRequest(this._nextId++, "archon/prompt", {
      sessionId: effectiveSessionId,
      text,
      contextFiles,
    });
  }

  /**
   * Answer an `archon/permissionRequest`.
   *
   * Rejects if the backend refuses the answer — a stale `requestId`, or one
   * the agent is no longer waiting on. That surfaces as an error in the panel
   * rather than as a button that silently did nothing.
   */
  async sendPermissionResponse(
    requestId: string,
    approved: boolean
  ): Promise<void> {
    await this._sendRequest(this._nextId++, "archon/permissionResponse", {
      sessionId: this._sessionId,
      requestId,
      approved,
    });
  }

  /** Cancel the in-flight turn. Resolves to whether one was actually running. */
  async cancel(): Promise<boolean> {
    const result = (await this._sendRequest(
      this._nextId++,
      "archon/cancel",
      { sessionId: this._sessionId }
    )) as { cancelled?: boolean } | undefined;
    return result?.cancelled === true;
  }

  /** Fetch session token and cost figures. */
  async getStatus(): Promise<SessionStatus> {
    return (await this._sendRequest(this._nextId++, "archon/status", {
      sessionId: this._sessionId,
    })) as SessionStatus;
  }

  /** Read one `archon/config` key. */
  async getConfig(key: string): Promise<unknown> {
    const result = (await this._sendRequest(this._nextId++, "archon/config", {
      key,
    })) as { value?: unknown } | undefined;
    return result?.value;
  }

  /** Write one `archon/config` key. */
  async setConfig(key: string, value: unknown): Promise<void> {
    await this._sendRequest(this._nextId++, "archon/config", { key, value });
  }

  /** Close the underlying transport and reset state to idle. */
  disconnect(): void {
    this._ws?.close();
    this._ws = null;
    if (this._child) {
      try {
        this._child.kill();
      } catch {
        // Process already gone; ignore.
      }
      this._child = null;
    }
    this._stdoutBuffer = "";
    this._rejectAllPending(new Error("Archon: disconnected"));
    this._state = "idle";
    this._sessionId = null;
  }

  // ── Private helpers ────────────────────────────────────────────────────────

  /**
   * Send `archon/initialize` and capture the returned sessionId. Promotes the
   * connection state to `connected` on success, `error` on failure.
   */
  private async _initialize(): Promise<void> {
    const id = this._nextId++;
    const params = {
      clientInfo: { name: "archon-vscode", version: "0.1.50" },
      capabilities: DEFAULT_CAPABILITIES,
    };

    const result = (await this._sendRequest(id, "archon/initialize", params)) as
      | { sessionId?: string }
      | undefined;

    const sessionId = result?.sessionId;
    if (typeof sessionId !== "string" || sessionId.length === 0) {
      this._state = "error";
      throw new Error("Archon: initialize succeeded but no sessionId returned");
    }
    this._sessionId = sessionId;
    this._state = "connected";
  }

  /**
   * Send a JSON-RPC request and await the matching response. Used for
   * request/response style calls (e.g. archon/initialize). Notifications
   * fired by the server (textDelta, turnComplete) are routed separately
   * via `_dispatchNotification`.
   */
  private async _sendRequest(
    id: number,
    method: string,
    params: unknown
  ): Promise<unknown> {
    return new Promise<unknown>((resolve, reject) => {
      this._pending.set(id, { resolve, reject });
      const payload = JSON.stringify({ jsonrpc: "2.0", id, method, params });
      try {
        this._send(payload);
      } catch (err) {
        this._pending.delete(id);
        reject(err instanceof Error ? err : new Error(String(err)));
      }
    });
  }

  private _send(payload: string): void {
    // Stdio transport: write a newline-delimited frame to the child's stdin.
    if (this._child?.stdin && !this._child.stdin.destroyed) {
      this._child.stdin.write(payload + "\n");
      return;
    }
    // WebSocket transport.
    if (this._ws && this._ws.readyState === WebSocket.OPEN) {
      this._ws.send(payload);
      return;
    }
    throw new Error("Archon: not connected");
  }

  private _rejectAllPending(err: Error): void {
    for (const { reject } of this._pending.values()) {
      reject(err);
    }
    this._pending.clear();
  }

  private _handleMessage(data: string): void {
    let msg: unknown;
    try {
      msg = JSON.parse(data) as unknown;
    } catch {
      return;
    }

    if (typeof msg !== "object" || msg === null) return;
    const obj = msg as Record<string, unknown>;

    // Response to a previous request (has `id` field)
    if ("id" in obj && typeof obj["id"] === "number") {
      const pending = this._pending.get(obj["id"]);
      if (pending) {
        this._pending.delete(obj["id"]);
        if ("error" in obj) {
          const err = obj["error"] as { message?: string } | undefined;
          pending.reject(new Error(err?.message ?? "JSON-RPC error"));
        } else {
          pending.resolve(obj["result"]);
        }
        return;
      }
    }

    // Notification (no `id` field)
    if (!("id" in obj) && "method" in obj && typeof obj["method"] === "string") {
      const notif = obj as { method: string; params: Record<string, unknown> };
      this._dispatchNotification(notif.method, notif.params ?? {});
    }
  }

  private _dispatchNotification(
    method: string,
    params: Record<string, unknown>
  ): void {
    switch (method) {
      case "archon/textDelta": {
        this.onTextDelta?.(str(params["text"]));
        break;
      }
      case "archon/thinkingDelta": {
        this.onThinkingDelta?.(str(params["thinking"]));
        break;
      }
      case "archon/toolCall": {
        this.onToolCall?.({
          toolUseId: str(params["toolUseId"]),
          name: str(params["name"]),
        });
        break;
      }
      case "archon/toolCallComplete": {
        this.onToolCallComplete?.({
          toolUseId: str(params["toolUseId"]),
          name: str(params["name"]),
          isError: params["isError"] === true,
          content: str(params["content"]),
        });
        break;
      }
      case "archon/permissionRequest": {
        this.onPermissionRequest?.({
          requestId: str(params["requestId"]),
          action: str(params["action"]),
          description: str(params["description"]),
        });
        break;
      }
      case "archon/permissionResolved": {
        this.onPermissionResolved?.({
          action: str(params["action"]),
          granted: params["granted"] === true,
          reason:
            typeof params["reason"] === "string" ? params["reason"] : undefined,
        });
        break;
      }
      case "archon/turnComplete": {
        const inputTokens =
          typeof params["inputTokens"] === "number" ? params["inputTokens"] : 0;
        const outputTokens =
          typeof params["outputTokens"] === "number"
            ? params["outputTokens"]
            : 0;
        this.onTurnComplete?.({ in: inputTokens, out: outputTokens });
        break;
      }
      case "archon/error": {
        this.onError?.(str(params["message"]));
        break;
      }
      default:
        break;
    }
  }
}

/** Coerce an untrusted JSON field to a string without throwing. */
function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}
