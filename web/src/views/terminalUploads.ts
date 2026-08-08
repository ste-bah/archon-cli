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
 */

import type { WebUploadResponse } from "../api/generated/web";

export interface UploadOutcome {
  /** Text to type into the terminal; empty when nothing was stored. */
  injection: string;
  /** Human-readable problem, if any. */
  error?: string;
}

export async function uploadDroppedFiles(files: File[]): Promise<UploadOutcome> {
  if (files.length === 0) {
    return { injection: "" };
  }

  const body = new FormData();
  for (const file of files) {
    body.append("file", file, file.name);
  }

  let response: Response;
  try {
    response = await fetch("/api/uploads/file", {
      method: "POST",
      headers: authHeaders(),
      body,
    });
  } catch (cause) {
    return { injection: "", error: `upload failed: ${String(cause)}` };
  }

  if (!response.ok) {
    // The handler answers 400 with a plain-text reason for anything malformed
    // or over a limit; that text is more useful than the status code.
    const reason = await response.text().catch(() => response.statusText);
    return { injection: "", error: reason || `upload failed (${response.status})` };
  }

  const result = (await response.json()) as WebUploadResponse;
  if (!result.accepted) {
    return { injection: "", error: result.policyReason };
  }

  return { injection: result.files.map((file) => `@${file.path} `).join("") };
}

/**
 * Mirrors `apiClient`'s scheme. Deliberately no `Content-Type`: the browser
 * must set it, because only it knows the multipart boundary.
 */
function authHeaders(): HeadersInit {
  const token = new URLSearchParams(window.location.search).get("token");
  return token ? { Authorization: `Bearer ${token}` } : {};
}
