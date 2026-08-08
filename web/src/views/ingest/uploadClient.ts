/**
 * Posting file bytes to `/api/uploads/file`.
 *
 * Shared by the Ingest tab's upload panel and the terminal pane's drop
 * handler; both need the same multipart POST and the same failure reporting,
 * and the endpoint answers 400 with a plain-text reason rather than JSON for
 * anything malformed or over a limit.
 */

import type { WebUploadedFile, WebUploadResponse } from "../../api/generated/web";

export interface UploadResult {
  files: WebUploadedFile[];
  error?: string;
}

export async function uploadFiles(files: File[]): Promise<UploadResult> {
  if (files.length === 0) return { files: [] };

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
    return { files: [], error: `upload failed: ${String(cause)}` };
  }

  if (!response.ok) {
    const reason = await response.text().catch(() => response.statusText);
    return { files: [], error: reason || `upload failed (${response.status})` };
  }

  const result = (await response.json()) as WebUploadResponse;
  return result.accepted
    ? { files: result.files }
    : { files: [], error: result.policyReason };
}

/**
 * Mirrors `apiClient`'s scheme. Deliberately no `Content-Type`: only the
 * browser knows the multipart boundary.
 */
function authHeaders(): HeadersInit {
  const token = new URLSearchParams(window.location.search).get("token");
  return token ? { Authorization: `Bearer ${token}` } : {};
}
