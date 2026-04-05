/**
 * ChatPanel — singleton webview panel hosting the Archon chat UI.
 *
 * Lifecycle:
 *   - Call `ChatPanel.createOrShow(context.extensionUri)` to open or reveal.
 *   - The panel disposes itself when the user closes the tab.
 *   - Inbound webview messages (type="prompt") are forwarded to the extension
 *     host for routing to `ConnectionManager`.
 */

import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs";

/** Identifier used to persist the webview panel across VS Code restarts. */
const VIEW_TYPE = "archonChat";

export class ChatPanel {
  /** The active panel instance, or undefined if none is open. */
  static current: ChatPanel | undefined;

  private readonly _panel: vscode.WebviewPanel;
  private readonly _extensionUri: vscode.Uri;
  private readonly _disposables: vscode.Disposable[] = [];

  /** Fired when the user sends a prompt from the webview. */
  readonly onDidReceivePrompt: vscode.Event<string>;
  private readonly _onDidReceivePromptEmitter: vscode.EventEmitter<string>;

  // ── Static factory ─────────────────────────────────────────────────────────

  /**
   * Open the chat panel, or bring an existing one to the front.
   *
   * @param extensionUri - `context.extensionUri` from the activate function.
   */
  static createOrShow(extensionUri: vscode.Uri): ChatPanel {
    const column = vscode.window.activeTextEditor
      ? vscode.ViewColumn.Beside
      : vscode.ViewColumn.One;

    if (ChatPanel.current) {
      ChatPanel.current._panel.reveal(column);
      return ChatPanel.current;
    }

    const panel = vscode.window.createWebviewPanel(
      VIEW_TYPE,
      "Archon Chat",
      column,
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [
          vscode.Uri.joinPath(extensionUri, "src", "chat"),
        ],
      }
    );

    ChatPanel.current = new ChatPanel(panel, extensionUri);
    return ChatPanel.current;
  }

  // ── Constructor ────────────────────────────────────────────────────────────

  private constructor(
    panel: vscode.WebviewPanel,
    extensionUri: vscode.Uri
  ) {
    this._panel = panel;
    this._extensionUri = extensionUri;

    this._onDidReceivePromptEmitter = new vscode.EventEmitter<string>();
    this.onDidReceivePrompt = this._onDidReceivePromptEmitter.event;

    this._panel.webview.html = this._buildHtml();

    this._panel.webview.onDidReceiveMessage(
      (message: { type: string; text?: string }) => {
        if (message.type === "prompt" && typeof message.text === "string") {
          this._onDidReceivePromptEmitter.fire(message.text);
        }
      },
      null,
      this._disposables
    );

    this._panel.onDidDispose(
      () => this.dispose(),
      null,
      this._disposables
    );
  }

  // ── Public API ─────────────────────────────────────────────────────────────

  /**
   * Post a message to the webview.
   *
   * @param message - Any JSON-serialisable object; the webview's message
   *                  handler dispatches on `message.type`.
   */
  sendMessage(message: Record<string, unknown>): void {
    this._panel.webview.postMessage(message);
  }

  /** Convenience helper: stream a text delta into the webview. */
  appendTextDelta(text: string): void {
    this.sendMessage({ type: "textDelta", text });
  }

  /** Convenience helper: signal turn completion to the webview. */
  notifyTurnComplete(tokensIn: number, tokensOut: number): void {
    this.sendMessage({ type: "turnComplete", tokensIn, tokensOut });
  }

  /** Convenience helper: display a system message (e.g. connection status). */
  showSystemMessage(text: string): void {
    this.sendMessage({ type: "systemMessage", text });
  }

  /** Convenience helper: display an error in the webview. */
  showError(message: string): void {
    this.sendMessage({ type: "error", message });
  }

  /** Reveal the panel without creating a new one. */
  reveal(column?: vscode.ViewColumn): void {
    this._panel.reveal(column);
  }

  /** Dispose the panel and clean up all event listeners. */
  dispose(): void {
    ChatPanel.current = undefined;
    this._panel.dispose();
    this._onDidReceivePromptEmitter.dispose();
    for (const d of this._disposables) {
      d.dispose();
    }
    this._disposables.length = 0;
  }

  // ── Private helpers ────────────────────────────────────────────────────────

  /** Load the bundled HTML template from disk and apply a nonce for CSP. */
  private _buildHtml(): string {
    const htmlPath = path.join(
      this._extensionUri.fsPath,
      "src",
      "chat",
      "webview.html"
    );

    let html: string;
    try {
      html = fs.readFileSync(htmlPath, "utf8");
    } catch {
      // Fallback for packaged extensions where the file may be embedded
      html = this._fallbackHtml();
    }

    // Inject VS Code webview origin CSP header
    const nonce = this._generateNonce();
    html = html.replace(
      /<script/g,
      `<script nonce="${nonce}"`
    );
    html = html.replace(
      /<\/head>/,
      `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'nonce-${nonce}'; style-src 'unsafe-inline';"/></head>`
    );

    return html;
  }

  private _generateNonce(): string {
    const chars =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let nonce = "";
    for (let i = 0; i < 32; i++) {
      nonce += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return nonce;
  }

  private _fallbackHtml(): string {
    return `<!DOCTYPE html><html><body>
      <p>Archon chat panel failed to load. Please reinstall the extension.</p>
    </body></html>`;
  }
}
