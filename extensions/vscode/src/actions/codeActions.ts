/**
 * ArchonCodeActionProvider — surfaces Archon actions in the editor lightbulb
 * and right-click context menu whenever the user has a text selection.
 *
 * Registered for all languages in `activate()` with:
 *   vscode.languages.registerCodeActionsProvider('*', new ArchonCodeActionProvider())
 */

import * as vscode from "vscode";
import { COMMANDS, CODE_ACTION_TITLES } from "../constants";

/** Maps each CODE_ACTION_TITLES entry to its command ID. */
const TITLE_TO_COMMAND: ReadonlyMap<string, string> = new Map([
  [CODE_ACTION_TITLES[0], COMMANDS.ASK_ARCHON],       // "Ask Archon"
  [CODE_ACTION_TITLES[1], COMMANDS.EXPLAIN_CODE],     // "Explain Code"
  [CODE_ACTION_TITLES[2], COMMANDS.FIX_ERROR],        // "Fix Error"
  [CODE_ACTION_TITLES[3], COMMANDS.GENERATE_TESTS],   // "Generate Tests"
]);

export class ArchonCodeActionProvider implements vscode.CodeActionProvider {
  static readonly providedCodeActionKinds = [vscode.CodeActionKind.Empty];

  /**
   * Returns Archon code actions when the user has a non-empty text selection.
   * Returns an empty array when nothing is selected (no lightbulb shown).
   */
  provideCodeActions(
    document: vscode.TextDocument,
    range: vscode.Range | vscode.Selection,
    _context: vscode.CodeActionContext,
    _token: vscode.CancellationToken
  ): vscode.CodeAction[] {
    // Only surface actions when text is selected
    const selection =
      range instanceof vscode.Selection ? range : new vscode.Selection(range.start, range.end);

    if (selection.isEmpty) {
      return [];
    }

    const selectedText = document.getText(selection);
    if (selectedText.trim().length === 0) {
      return [];
    }

    return CODE_ACTION_TITLES.map((title) => {
      const commandId = TITLE_TO_COMMAND.get(title) ?? COMMANDS.ASK_ARCHON;
      const action = new vscode.CodeAction(title, vscode.CodeActionKind.Empty);
      action.command = {
        command: commandId,
        title,
        arguments: [selectedText, document.uri],
      };
      return action;
    });
  }
}
