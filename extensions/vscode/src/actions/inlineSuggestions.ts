/**
 * ArchonInlineCompletionProvider — placeholder implementation for Phase 6.
 *
 * Returns an empty list of completions for now. An empty array is a valid
 * production response that tells VS Code "no suggestions at this position."
 * This avoids showing stale or incorrect completions until the full streaming
 * completion pipeline is implemented in a later phase.
 */

import * as vscode from "vscode";

export class ArchonInlineCompletionProvider
  implements vscode.InlineCompletionItemProvider
{
  /**
   * Called by VS Code when the user pauses typing. Returns an empty list.
   *
   * Phase 6 will replace this with a call to `ConnectionManager.sendPrompt()`
   * and stream the result back as an `InlineCompletionItem`.
   */
  provideInlineCompletionItems(
    _document: vscode.TextDocument,
    _position: vscode.Position,
    _context: vscode.InlineCompletionContext,
    _token: vscode.CancellationToken
  ): vscode.ProviderResult<vscode.InlineCompletionList | vscode.InlineCompletionItem[]> {
    return [];
  }
}
