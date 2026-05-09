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
} from "../types";

/** Token-usage summary delivered when a turn finishes. */
export interface TurnTokens {
  in: number;
  out: number;
}

/** Default capabilities advertised by the VS Code extension during initialize. */
const DEFAULT_CAPABILITIES: IdeCapabilities = {
  inlineCompletion: false,
  toolExecution: false,
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
  public onTurnComplete: ((tokens: TurnTokens) => void) | null = null;

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
   */
  async connectStdio(binaryPath: string, _mode: ConnectionMode): Promise<void> {
    this._state = "connecting";

    // Dynamic require keeps the `child_process` import out of webview bundles.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { spawn } = require("child_process") as typeof import("child_process");

    await new Promise<void>((resolve, reject) => {
      const child = spawn(binaryPath, ["ide-stdio"], {
        stdio: ["pipe", "pipe", "inherit"],
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

      child.on("error", (err: Error) => {
        this._state = "error";
        reject(new Error(`Archon: failed to spawn binary — ${err.message}`));
      });

      child.on("exit", () => {
        if (this._state === "connected") {
          this._state = "idle";
        }
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
    const id = this._nextId++;
    const payload = JSON.stringify({
      jsonrpc: "2.0",
      id,
      method: "archon/prompt",
      params: { sessionId: effectiveSessionId, text, contextFiles },
    });
    this._send(payload);
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
        const text = typeof params["text"] === "string" ? params["text"] : "";
        this.onTextDelta?.(text);
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
      default:
        break;
    }
  }
}
