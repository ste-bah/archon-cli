/**
 * Shared TypeScript interfaces mirroring the archon-sdk-ts protocol types.
 * This file intentionally has NO dependency on the `vscode` module so it can
 * be imported by plain-Node unit tests without a VS Code runtime.
 */

/** Capabilities advertised by the IDE client during initialization. */
export interface IdeCapabilities {
  inlineCompletion: boolean;
  toolExecution: boolean;
  diff: boolean;
  terminal: boolean;
}

/**
 * JSON-RPC 2.0 request frame for `archon/initialize`.
 * Required fields: jsonrpc, id, method, params.
 */
export interface InitializeMessage {
  jsonrpc: "2.0";
  id: number;
  method: "archon/initialize";
  params: {
    clientInfo: { name: string; version: string };
    capabilities: IdeCapabilities;
  };
}

/**
 * JSON-RPC 2.0 request frame for `archon/prompt`.
 * Required fields: jsonrpc, id, method, params.
 */
export interface PromptMessage {
  jsonrpc: "2.0";
  id: number;
  method: "archon/prompt";
  params: {
    sessionId: string;
    text: string;
    contextFiles?: string[];
  };
}

/** Configuration for the WebSocket transport. */
export interface WsConnectionConfig {
  /** WebSocket endpoint URL. Default: ws://localhost:8420/ws/ide */
  url: string;
  /** Optional bearer token for authenticated deployments. */
  token?: string;
}

/** Default WebSocket configuration used when none is provided. */
export const DEFAULT_WS_CONFIG: WsConnectionConfig = {
  url: "ws://localhost:8420/ws/ide",
};

/**
 * Lifecycle state of the connection to the Archon backend.
 *  - idle        — not yet started
 *  - connecting  — transport being established
 *  - connected   — ready to send prompts
 *  - error       — last connection attempt failed
 */
export type ConnectionState = "idle" | "connecting" | "connected" | "error";

/**
 * Returns a human-readable status bar label for a given connection state.
 *
 * @param state - Current ConnectionState
 * @returns Short label suitable for a VS Code status bar item
 */
export function formatStatusText(state: ConnectionState): string {
  switch (state) {
    case "idle":
      return "$(circle-slash) Archon: idle";
    case "connecting":
      return "$(sync~spin) Archon: connecting…";
    case "connected":
      return "$(check) Archon: connected";
    case "error":
      return "$(error) Archon: error";
  }
}
