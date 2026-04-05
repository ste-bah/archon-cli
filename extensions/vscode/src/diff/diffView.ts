/**
 * showDiff — opens VS Code's built-in diff editor to compare two strings.
 *
 * Creates temporary URIs using the `untitled:` scheme so no files are
 * written to disk. The function resolves to `true` when the user saves the
 * right-hand document (interpreted as accepting the change) or `false` if
 * the diff editor is closed without saving.
 *
 * @param oldContent - Original (left-hand) text shown in the diff editor.
 * @param newContent - Proposed (right-hand) text shown in the diff editor.
 * @param title      - Label shown in the diff editor tab.
 * @returns Promise resolving to `true` if the user accepted the diff.
 */

import * as vscode from "vscode";

export async function showDiff(
  oldContent: string,
  newContent: string,
  title = "Archon Suggestion"
): Promise<boolean> {
  // Build untitled URIs that uniquely identify this diff session.
  const timestamp = Date.now();
  const oldUri = vscode.Uri.parse(`untitled:archon-old-${timestamp}`);
  const newUri = vscode.Uri.parse(`untitled:archon-new-${timestamp}`);

  // Populate both documents using workspace edits so no disk I/O is required.
  const edit = new vscode.WorkspaceEdit();
  edit.insert(oldUri, new vscode.Position(0, 0), oldContent);
  edit.insert(newUri, new vscode.Position(0, 0), newContent);
  await vscode.workspace.applyEdit(edit);

  // Open the diff view.
  await vscode.commands.executeCommand(
    "vscode.diff",
    oldUri,
    newUri,
    title
  );

  // Watch for the user saving the right-hand document as the acceptance signal.
  return new Promise<boolean>((resolve) => {
    let accepted = false;

    const saveListener = vscode.workspace.onDidSaveTextDocument((doc) => {
      if (doc.uri.toString() === newUri.toString()) {
        accepted = true;
        cleanup();
        resolve(true);
      }
    });

    const closeListener = vscode.window.onDidChangeVisibleTextEditors(() => {
      const isStillOpen = vscode.window.visibleTextEditors.some(
        (ed) => ed.document.uri.toString() === newUri.toString()
      );
      if (!isStillOpen && !accepted) {
        cleanup();
        resolve(false);
      }
    });

    function cleanup(): void {
      saveListener.dispose();
      closeListener.dispose();
    }
  });
}
