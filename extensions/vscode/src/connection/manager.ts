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

import { ConnectionMode } from "../constants";
import {
  ConnectionState,
  WsConnectionConfig,
  DEFAULT_WS_CONFIG,
} from "../types";

/** Token-usage summary delivered when a turn finishes. */
export interface TurnTokens {
  in: number;
  out: number;
}

export class ConnectionManager {
  private _state: ConnectionState = "idle";
  private _ws: WebSocket | null = null;
  private _sessionId: string | null = null;
  private _nextId = 1;

  // Public event callbacks
  public onTextDelta: ((text: string) => void) | null = null;
  public onTurnComplete: ((tokens: TurnTokens) => void) | null = null;

  // ── Public API ─────────────────────────────────────────────────────────────

  /** Returns the current connection state. */
  getState(): ConnectionState {
    return this._state;
  }

  /**
   * Establish a WebSocket connection and perform the initialization handshake.
   *
   * @param config - WebSocket endpoint configuration. Defaults to localhost:8420.
   */
  async connect(config: WsConnectionConfig = DEFAULT_WS_CONFIG): Promise<void> {
    this._state = "connecting";

    return new Promise<void>((resolve, reject) => {
      // Use global WebSocket — available in VS Code extension host (Node ≥ 22)
      // and in browser-based webview contexts.
      const ws = new WebSocket(config.url);
      this._ws = ws;

      const headers: Record<string, string> = {};
      if (config.token) {
        headers["Authorization"] = `Bearer ${config.token}`;
      }

      ws.onopen = () => {
        this._state = "connected";
        resolve();
      };

      ws.onerror = () => {
        this._state = "error";
        reject(new Error("Archon: WebSocket connection failed"));
      };

      ws.onclose = () => {
        if (this._state === "connected") {
          this._state = "idle";
        }
      };

      ws.onmessage = (event: MessageEvent<string>) => {
        this._handleMessage(event.data);
      };
    });
  }

  /**
   * Connect via Archon's stdio transport.
   *
   * The connection manager spawns the Archon binary as a child process and
   * communicates via newline-delimited JSON-RPC on stdin/stdout.
   *
   * @param binaryPath - Path to the `archon` executable.
   * @param mode - Must be ConnectionMode.Stdio (validated at call site).
   */
  async connectStdio(binaryPath: string, _mode: ConnectionMode): Promise<void> {
    this._state = "connecting";

    // Dynamic require keeps the `child_process` import out of webview bundles.
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { spawn } = require("child_process") as typeof import("child_process");

    return new Promise<void>((resolve, reject) => {
      const child = spawn(binaryPath, ["--ide-mode"], {
        stdio: ["pipe", "pipe", "inherit"],
      });

      let buffer = "";

      child.stdout?.on("data", (chunk: Buffer) => {
        buffer += chunk.toString("utf8");
        const lines = buffer.split("\n");
        buffer = lines.pop() ?? "";
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

      // Resolve once the process is running; initialization is caller's concern.
      child.on("spawn", () => {
        this._state = "connected";
        resolve();
      });
    });
  }

  /**
   * Send an `archon/prompt` JSON-RPC request to the connected backend.
   *
   * @param sessionId - Active session identifier from the initialize handshake.
   * @param text - User prompt text.
   * @param contextFiles - Optional list of workspace-relative file paths.
   */
  async sendPrompt(
    sessionId: string,
    text: string,
    contextFiles?: string[]
  ): Promise<void> {
    this._sessionId = sessionId;
    const id = this._nextId++;
    const payload = JSON.stringify({
      jsonrpc: "2.0",
      id,
      method: "archon/prompt",
      params: { sessionId, text, contextFiles },
    });
    this._send(payload);
  }

  /** Close the underlying transport and reset state to idle. */
  disconnect(): void {
    this._ws?.close();
    this._ws = null;
    this._state = "idle";
    this._sessionId = null;
  }

  // ── Private helpers ────────────────────────────────────────────────────────

  private _send(payload: string): void {
    if (!this._ws || this._ws.readyState !== WebSocket.OPEN) {
      throw new Error("Archon: not connected");
    }
    this._ws.send(payload);
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
