/**
 * Extension-wide constants: command IDs, configuration keys, and shared enumerations.
 */

/** All VS Code command IDs contributed by the Archon extension. */
export const COMMANDS = {
  OPEN_CHAT: "archon.openChat",
  ASK_ARCHON: "archon.askArchon",
  EXPLAIN_CODE: "archon.explainCode",
  FIX_ERROR: "archon.fixError",
  GENERATE_TESTS: "archon.generateTests",
  RECONNECT: "archon.reconnect",
} as const;

/** VS Code configuration keys (section: "archon"). */
export const CONFIG_KEY_CONNECTION_MODE = "archon.connectionMode";
export const CONFIG_KEY_BINARY_PATH = "archon.binaryPath";
export const CONFIG_KEY_WEBSOCKET_URL = "archon.websocketUrl";

/**
 * Human-readable titles for code actions shown in the editor context menu.
 * Consumed by ArchonCodeActionProvider to build vscode.CodeAction objects.
 */
export const CODE_ACTION_TITLES: readonly string[] = [
  "Ask Archon",
  "Explain Code",
  "Fix Error",
  "Generate Tests",
];

/** Transport mode used to reach the Archon backend. */
export enum ConnectionMode {
  Stdio = "stdio",
  WebSocket = "websocket",
}
