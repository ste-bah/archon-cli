/**
 * Archon VS Code Extension — main entry point.
 *
 * `activate` is called once by VS Code when any activation event fires.
 * `deactivate` is called when the extension is unloaded.
 *
 * Responsibilities:
 *  - Register all 6 contributed commands.
 *  - Register ArchonCodeActionProvider for all languages.
 *  - Register ArchonInlineCompletionProvider for all languages.
 *  - Create and maintain the status bar item that reflects connection state.
 *  - Wire the chat panel to the ConnectionManager for streaming output.
 */

import * as vscode from "vscode";
import { COMMANDS, CONFIG_KEY_CONNECTION_MODE, CONFIG_KEY_BINARY_PATH, CONFIG_KEY_WEBSOCKET_URL, ConnectionMode } from "./constants";
import { formatStatusText, WsConnectionConfig } from "./types";
import { ConnectionManager } from "./connection/manager";
import { ChatPanel } from "./chat/panel";
import { ArchonCodeActionProvider } from "./actions/codeActions";
import { ArchonInlineCompletionProvider } from "./actions/inlineSuggestions";

/** Singleton connection manager shared across the extension session. */
let connectionManager: ConnectionManager | null = null;

/** Status bar item showing the current connection state. */
let statusBarItem: vscode.StatusBarItem | null = null;

// ── Activate ──────────────────────────────────────────────────────────────────

export async function activate(
  context: vscode.ExtensionContext
): Promise<void> {
  // ── Status bar ─────────────────────────────────────────────────────────────
  statusBarItem = vscode.window.createStatusBarItem(
    vscode.StatusBarAlignment.Right,
    100
  );
  statusBarItem.command = COMMANDS.OPEN_CHAT;
  statusBarItem.tooltip = "Click to open Archon Chat";
  statusBarItem.text = formatStatusText("idle");
  statusBarItem.show();
  context.subscriptions.push(statusBarItem);

  // ── Connection manager ─────────────────────────────────────────────────────
  connectionManager = new ConnectionManager();

  // ── Command: archon.openChat ───────────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand(COMMANDS.OPEN_CHAT, () => {
      const panel = ChatPanel.createOrShow(context.extensionUri);
      panel.onDidReceivePrompt(async (text) => {
        await sendPromptToChatPanel(panel, text);
      });
    })
  );

  // ── Command: archon.askArchon ──────────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand(
      COMMANDS.ASK_ARCHON,
      async (selectedText?: string) => {
        const text =
          selectedText ??
          vscode.window.activeTextEditor?.document.getText(
            vscode.window.activeTextEditor.selection
          );

        if (!text || text.trim().length === 0) {
          await vscode.window.showWarningMessage(
            "Archon: Select some text before asking."
          );
          return;
        }

        const panel = ChatPanel.createOrShow(context.extensionUri);
        panel.onDidReceivePrompt(async (prompt) => {
          await sendPromptToChatPanel(panel, prompt);
        });
        await sendPromptToChatPanel(panel, text);
      }
    )
  );

  // ── Command: archon.explainCode ────────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand(
      COMMANDS.EXPLAIN_CODE,
      async (selectedText?: string) => {
        const code =
          selectedText ??
          vscode.window.activeTextEditor?.document.getText(
            vscode.window.activeTextEditor.selection
          ) ??
          "";
        const panel = ChatPanel.createOrShow(context.extensionUri);
        panel.onDidReceivePrompt(async (prompt) => {
          await sendPromptToChatPanel(panel, prompt);
        });
        if (code.trim().length > 0) {
          await sendPromptToChatPanel(panel, `Explain this code:\n\`\`\`\n${code}\n\`\`\``);
        }
      }
    )
  );

  // ── Command: archon.fixError ───────────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand(
      COMMANDS.FIX_ERROR,
      async (selectedText?: string) => {
        const code =
          selectedText ??
          vscode.window.activeTextEditor?.document.getText(
            vscode.window.activeTextEditor.selection
          ) ??
          "";
        const panel = ChatPanel.createOrShow(context.extensionUri);
        panel.onDidReceivePrompt(async (prompt) => {
          await sendPromptToChatPanel(panel, prompt);
        });
        if (code.trim().length > 0) {
          await sendPromptToChatPanel(panel, `Fix this error:\n\`\`\`\n${code}\n\`\`\``);
        }
      }
    )
  );

  // ── Command: archon.generateTests ─────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand(
      COMMANDS.GENERATE_TESTS,
      async (selectedText?: string) => {
        const code =
          selectedText ??
          vscode.window.activeTextEditor?.document.getText(
            vscode.window.activeTextEditor.selection
          ) ??
          "";
        const panel = ChatPanel.createOrShow(context.extensionUri);
        panel.onDidReceivePrompt(async (prompt) => {
          await sendPromptToChatPanel(panel, prompt);
        });
        if (code.trim().length > 0) {
          await sendPromptToChatPanel(panel, `Generate unit tests for:\n\`\`\`\n${code}\n\`\`\``);
        }
      }
    )
  );

  // ── Command: archon.reconnect ──────────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand(COMMANDS.RECONNECT, async () => {
      connectionManager?.disconnect();
      updateStatusBar("connecting");
      try {
        await connectFromConfig();
        updateStatusBar("connected");
        ChatPanel.current?.showSystemMessage("Reconnected to Archon.");
      } catch (err) {
        updateStatusBar("error");
        const msg = err instanceof Error ? err.message : String(err);
        await vscode.window.showErrorMessage(`Archon: reconnect failed — ${msg}`);
      }
    })
  );

  // ── Code actions ───────────────────────────────────────────────────────────
  context.subscriptions.push(
    vscode.languages.registerCodeActionsProvider(
      { scheme: "*", language: "*" },
      new ArchonCodeActionProvider(),
      { providedCodeActionKinds: ArchonCodeActionProvider.providedCodeActionKinds }
    )
  );

  // ── Inline completions ─────────────────────────────────────────────────────
  context.subscriptions.push(
    vscode.languages.registerInlineCompletionItemProvider(
      { scheme: "*", language: "*" },
      new ArchonInlineCompletionProvider()
    )
  );

  // ── Auto-connect on startup ────────────────────────────────────────────────
  try {
    await connectFromConfig();
    updateStatusBar("connected");
  } catch {
    // Non-fatal: server may not be running yet. User can reconnect manually.
    updateStatusBar("error");
  }
}

// ── Deactivate ────────────────────────────────────────────────────────────────

export function deactivate(): void {
  connectionManager?.disconnect();
  connectionManager = null;
  ChatPanel.current?.dispose();
  statusBarItem?.dispose();
  statusBarItem = null;
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/** Read VS Code config and establish the appropriate transport. */
async function connectFromConfig(): Promise<void> {
  const config = vscode.workspace.getConfiguration();
  const mode = config.get<string>(CONFIG_KEY_CONNECTION_MODE, "stdio") as ConnectionMode;

  if (!connectionManager) return;

  if (mode === ConnectionMode.WebSocket) {
    const url = config.get<string>(
      CONFIG_KEY_WEBSOCKET_URL,
      "ws://localhost:8420/ws/ide"
    );
    const wsConfig: WsConnectionConfig = { url };
    await connectionManager.connect(wsConfig);
  } else {
    const binaryPath = config.get<string>(CONFIG_KEY_BINARY_PATH, "archon");
    await connectionManager.connectStdio(binaryPath, ConnectionMode.Stdio);
  }
}

/** Update status bar text based on connection state. */
function updateStatusBar(
  state: "idle" | "connecting" | "connected" | "error"
): void {
  if (statusBarItem) {
    statusBarItem.text = formatStatusText(state);
  }
}

/**
 * Ensure the connection is up, send a prompt, and stream deltas back
 * into the given `ChatPanel`.
 */
async function sendPromptToChatPanel(
  panel: ChatPanel,
  text: string
): Promise<void> {
  if (!connectionManager) return;

  if (connectionManager.getState() !== "connected") {
    panel.showSystemMessage("Archon is not connected. Use Archon: Reconnect.");
    return;
  }

  // Route streaming callbacks through the chat panel
  connectionManager.onTextDelta = (delta: string) => {
    panel.appendTextDelta(delta);
  };

  connectionManager.onTurnComplete = (tokens: { in: number; out: number }) => {
    panel.notifyTurnComplete(tokens.in, tokens.out);
    updateStatusBar("connected");
  };

  updateStatusBar("connecting");

  try {
    // Use a placeholder session ID; real initialization happens in connectFromConfig.
    await connectionManager.sendPrompt("default-session", text);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    panel.showError(msg);
    updateStatusBar("error");
  }
}
