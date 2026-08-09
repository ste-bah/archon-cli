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
import { COMMANDS, CONFIG_KEY_CONNECTION_MODE, CONFIG_KEY_BINARY_PATH, CONFIG_KEY_WEBSOCKET_URL, ConnectionMode, PERMISSION_MODES } from "./constants";
import { formatStatusText, WsConnectionConfig } from "./types";
import { ConnectionManager } from "./connection/manager";
import { ChatPanel } from "./chat/panel";
import type { SessionStatus } from "./types";
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
      attachPanel(ChatPanel.createOrShow(context.extensionUri));
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

        const panel = attachPanel(ChatPanel.createOrShow(context.extensionUri));
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
        const panel = attachPanel(ChatPanel.createOrShow(context.extensionUri));
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
        const panel = attachPanel(ChatPanel.createOrShow(context.extensionUri));
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
        const panel = attachPanel(ChatPanel.createOrShow(context.extensionUri));
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

  // ── Command: archon.showStatus ─────────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand(COMMANDS.SHOW_STATUS, async () => {
      if (!connectionManager) return;
      try {
        const status = await connectionManager.getStatus();
        await vscode.window.showInformationMessage(
          `Archon: ${formatStatus(status)}`
        );
      } catch (err) {
        await vscode.window.showErrorMessage(
          `Archon: status unavailable — ${errText(err)}`
        );
      }
    })
  );

  // ── Command: archon.setPermissionMode ──────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand(COMMANDS.SET_PERMISSION_MODE, async () => {
      if (!connectionManager) return;
      const chosen = await vscode.window.showQuickPick([...PERMISSION_MODES], {
        title: "Archon permission mode for this session",
      });
      if (!chosen) return;
      try {
        await connectionManager.setConfig("permissionMode", chosen);
        ChatPanel.current?.showSystemMessage(`Permission mode: ${chosen}`);
      } catch (err) {
        await vscode.window.showErrorMessage(
          `Archon: could not set permission mode — ${errText(err)}`
        );
      }
    })
  );

  // ── Command: archon.cancel ─────────────────────────────────────────────────
  context.subscriptions.push(
    vscode.commands.registerCommand(COMMANDS.CANCEL, async () => {
      if (!connectionManager) return;
      try {
        const cancelled = await connectionManager.cancel();
        ChatPanel.current?.showSystemMessage(
          cancelled ? "Stopped." : "Nothing was running to stop."
        );
      } catch (err) {
        await vscode.window.showErrorMessage(
          `Archon: cancel failed — ${errText(err)}`
        );
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
    // `archon serve`'s /ws/ide endpoint is request/response only: it has no
    // way to push a frame the client did not ask for, so a prompt sent over it
    // would run with no `archon/textDelta` able to travel back. The backend
    // refuses those methods explicitly rather than pretending; say so here
    // too, so the failure is legible before the first prompt rather than after.
    void vscode.window.showWarningMessage(
      "Archon: websocket mode connects but cannot run prompts — the /ws/ide endpoint " +
        "cannot stream notifications. Set archon.connectionMode to \"stdio\"."
    );
    const url = config.get<string>(
      CONFIG_KEY_WEBSOCKET_URL,
      "ws://localhost:8420/ws/ide"
    );
    const wsConfig: WsConnectionConfig = { url };
    await connectionManager.connect(wsConfig);
  } else {
    const binaryPath = config.get<string>(CONFIG_KEY_BINARY_PATH, "archon");
    // First folder only: the backend is a single-workspace process, and a
    // multi-root window has no one right answer to give it.
    const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    await connectionManager.connectStdio(
      binaryPath,
      ConnectionMode.Stdio,
      workspaceRoot
    );
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
 * Wire a freshly created panel to the connection, once.
 *
 * Panels created by the various commands used to register a prompt listener
 * each time the command ran, so opening the chat twice sent every prompt
 * twice. `ChatPanel.createOrShow` is a singleton, so the wiring is keyed off
 * the instance and only ever done for a panel that has not been seen before.
 */
let wiredPanel: ChatPanel | undefined;

function attachPanel(panel: ChatPanel): ChatPanel {
  if (wiredPanel === panel) return panel;
  wiredPanel = panel;

  panel.onDidReceivePrompt(async (text) => {
    await sendPromptToChatPanel(panel, text);
  });

  panel.onDidDecidePermission(async (decision) => {
    if (!connectionManager) return;
    try {
      await connectionManager.sendPermissionResponse(
        decision.requestId,
        decision.approved
      );
    } catch (err) {
      // The backend refuses an answer it is not waiting on. Surfacing that is
      // the whole point: a button that silently does nothing is how a
      // permission prompt stops meaning anything.
      panel.showError(
        `Archon: permission decision was not accepted — ${errText(err)}`
      );
    }
  });

  panel.onDidRequestCancel(async () => {
    if (!connectionManager) return;
    try {
      const cancelled = await connectionManager.cancel();
      panel.showSystemMessage(
        cancelled ? "Stopped." : "Nothing was running to stop."
      );
      if (!cancelled) panel.notifyTurnComplete(0, 0);
    } catch (err) {
      panel.showError(`Archon: cancel failed — ${errText(err)}`);
    }
  });

  if (connectionManager) {
    routeNotificationsTo(panel);
  }
  return panel;
}

/** Point every server notification at `panel`. */
function routeNotificationsTo(panel: ChatPanel): void {
  if (!connectionManager) return;

  connectionManager.onTextDelta = (delta) => panel.appendTextDelta(delta);
  connectionManager.onThinkingDelta = (delta) =>
    panel.appendThinkingDelta(delta);
  connectionManager.onToolCall = (call) =>
    panel.showToolCall(call.toolUseId, call.name);
  connectionManager.onToolCallComplete = (result) =>
    panel.showToolResult(
      result.toolUseId,
      result.name,
      result.isError,
      result.content
    );
  connectionManager.onPermissionRequest = (request) =>
    panel.requestPermission(
      request.requestId,
      request.action,
      request.description
    );
  connectionManager.onPermissionResolved = (resolved) =>
    panel.resolvePermission(resolved.action, resolved.granted, resolved.reason);
  connectionManager.onError = (message) => panel.showError(message);
  connectionManager.onTurnComplete = (tokens) => {
    panel.notifyTurnComplete(tokens.in, tokens.out);
    updateStatusBar("connected");
  };
}

/** Ensure the connection is up and send a prompt. */
async function sendPromptToChatPanel(
  panel: ChatPanel,
  text: string
): Promise<void> {
  if (!connectionManager) return;

  if (connectionManager.getState() !== "connected") {
    panel.showSystemMessage("Archon is not connected. Use Archon: Reconnect.");
    // Without this the panel stays disabled waiting for a turn that was never
    // started, and the user cannot even retype the prompt.
    panel.notifyTurnComplete(0, 0);
    return;
  }

  routeNotificationsTo(panel);
  updateStatusBar("connecting");

  try {
    // Real session ID was captured during the initialize handshake in
    // connectFromConfig; sendPrompt prefers it over the legacy argument.
    const sessionId = connectionManager.getSessionId() ?? "default-session";
    await connectionManager.sendPrompt(sessionId, text);
  } catch (err) {
    panel.showError(errText(err));
    updateStatusBar("error");
  }
}

/** Render a status result, including its explicit "no reading yet" case. */
function formatStatus(status: SessionStatus): string {
  if (status.unavailable) {
    return `Model ${status.model ?? "unknown"} — ${status.unavailable}`;
  }
  const cost = status.cost === undefined ? "?" : status.cost.toFixed(4);
  return `Model ${status.model ?? "unknown"} — ${status.inputTokens ?? 0} in / ${status.outputTokens ?? 0} out, $${cost}`;
}

function errText(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}
