/**
 * ArchonTerminal — thin wrapper around a vscode.Terminal instance.
 *
 * Provides a stable handle that the extension can use to run shell commands on
 * behalf of the user (e.g., running generated tests or applying scaffolding).
 */

import * as vscode from "vscode";

export class ArchonTerminal {
  private _terminal: vscode.Terminal;

  /**
   * @param name - Label shown in the terminal tab. Defaults to "Archon".
   */
  constructor(name = "Archon") {
    this._terminal = vscode.window.createTerminal({
      name,
      iconPath: new vscode.ThemeIcon("robot"),
    });
  }

  /**
   * Send a shell command to the terminal and show it.
   *
   * @param command - Shell command string; executed immediately on Enter.
   */
  execute(command: string): void {
    this._terminal.show(true /* preserveFocus */);
    this._terminal.sendText(command, true /* addNewLine */);
  }

  /** Reveal the terminal without executing a command. */
  show(): void {
    this._terminal.show(true);
  }

  /** Dispose the underlying terminal instance. */
  dispose(): void {
    this._terminal.dispose();
  }
}
