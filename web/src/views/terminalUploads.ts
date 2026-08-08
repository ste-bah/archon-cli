/**
 * Drag-and-drop attachments for the terminal pane.
 *
 * A terminal emulator has no notion of a file drop — bytes go in, bytes come
 * out. So the drop is resolved on the server side instead: the file is posted
 * to `/api/uploads/file`, which writes it to a staging directory and answers
 * with an absolute path, and that path is then *typed* into the PTY as
 * `@<path>`.
 *
 * `@<path>` is the TUI's own attachment convention, not something invented
 * here — the `/files` picker injects exactly that string into the input line
 * (`crates/archon-tui/src/event_loop/input.rs`). So the browser gains
 * drag-and-drop while the TUI learns nothing about the web, and the feature
 * cannot drift out of step with however attachments are handled next.
 *
 * The POST itself is [`uploadFiles`], shared with the Ingest tab's upload
 * panel; only the "what to do with the returned path" part differs.
 */

import { uploadFiles } from "./ingest/uploadClient";

export interface UploadOutcome {
  /** Text to type into the terminal; empty when nothing was stored. */
  injection: string;
  /** Human-readable problem, if any. */
  error?: string;
}

export async function uploadDroppedFiles(files: File[]): Promise<UploadOutcome> {
  const result = await uploadFiles(files);
  if (result.error) {
    return { injection: "", error: result.error };
  }
  return { injection: result.files.map((file) => `@${file.path} `).join("") };
}
