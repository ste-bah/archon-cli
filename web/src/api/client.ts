import type {
  ApiStatus,
  CorpusSearchResponse,
  CorpusSummary,
  CorpusSourcePreview,
  CognitiveWebSummary,
  EvidenceGraphSummary,
  EffectiveConfigSummary,
  EffectivePolicySummary,
  LearningSummary,
  MetricsSummary,
  PipelineSummary,
  SettingsSummary,
  WebThemeProfileEnvelope,
  WebThemeProfileSaveRequest,
  WebActionRequest,
  WebActionResponse,
  WebAuthSession,
  WebChatHistoryResponse,
  WebChatSubmitRequest,
  WebChatSubmitResponse,
  WebIngestRunRequest,
  WebIngestRunResponse,
  WebIngestSummary,
  WorkflowControlRequest,
  WorkflowControlResponse,
  WorkflowEventPreview,
  WorkflowRunDetail,
  WorkflowWebSummary,
  WebAgentActivitySnapshot,
  WebBoardActivity,
  WebBoardHistory,
  WebBoardItems,
  WebBoardRunList,
  WebKbCreateRequest,
  WebKbCreateResponse,
  WebDocDeleteRequest,
  WebDocDeleteResponse,
  WebIndexControlRequest,
  WebIndexControlResponse,
  WebLiveCursorExpired,
  WebLiveSnapshot,
  WebUploadIntent,
  WebUploadIntentResponse,
  WebUploadPolicy,
  WorldInspectionSummary,
} from "./generated/web";

const jsonHeaders = {
  Accept: "application/json",
};

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(path, {
    headers: authHeaders(),
    credentials: "same-origin",
  });
  if (!response.ok) {
    throw new Error(`${path} failed with ${response.status}`);
  }
  return (await response.json()) as T;
}

/**
 * Fetch a binary response body.
 *
 * Separate from `getJson` because the corpus byte endpoint answers with a
 * document, not JSON, and the caller hands the buffer straight to PDF.js
 * rather than letting the browser navigate to it.
 */
async function getBytes(path: string, accept: string): Promise<ArrayBuffer> {
  const response = await fetch(path, {
    headers: authHeaders(accept),
    credentials: "same-origin",
  });
  if (!response.ok) {
    throw new Error(`${path} failed with ${response.status}`);
  }
  return await response.arrayBuffer();
}

async function postJson<T>(path: string, body: unknown, timeoutMs?: number): Promise<T> {
  const controller = timeoutMs ? new AbortController() : undefined;
  const timeout = controller
    ? window.setTimeout(() => controller.abort(), timeoutMs)
    : undefined;
  try {
    const response = await fetch(path, {
      method: "POST",
      headers: { ...authHeaders(), "Content-Type": "application/json" },
      credentials: "same-origin",
      body: JSON.stringify(body),
      signal: controller?.signal,
    });
    if (!response.ok) {
      throw new Error(`${path} failed with ${response.status}`);
    }
    return (await response.json()) as T;
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") {
      throw new Error(`${path} timed out after ${Math.round((timeoutMs ?? 0) / 1000)}s`);
    }
    throw error;
  } finally {
    if (timeout !== undefined) {
      window.clearTimeout(timeout);
    }
  }
}

async function streamSseJson<T>(
  path: string,
  onEvent: (value: T) => void,
  signal: AbortSignal,
): Promise<void> {
  const response = await fetch(path, {
    headers: authHeaders(),
    credentials: "same-origin",
    signal,
  });
  if (!response.ok) {
    throw new Error(`${path} failed with ${response.status}`);
  }
  const reader = response.body?.getReader();
  if (!reader) {
    throw new Error(`${path} did not provide a stream body`);
  }
  const decoder = new TextDecoder();
  let buffer = "";
  while (!signal.aborted) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    buffer += decoder.decode(value, { stream: true });
    buffer = drainSseBuffer(buffer, onEvent);
  }
}

function drainSseBuffer<T>(buffer: string, onEvent: (value: T) => void): string {
  let remaining = buffer;
  let boundary = remaining.indexOf("\n\n");
  while (boundary >= 0) {
    const block = remaining.slice(0, boundary);
    remaining = remaining.slice(boundary + 2);
    const value = parseSseJson<T>(block);
    if (value !== undefined) {
      onEvent(value);
    }
    boundary = remaining.indexOf("\n\n");
  }
  return remaining;
}

function parseSseJson<T>(block: string): T | undefined {
  const data = block
    .split(/\r?\n/)
    .filter((line) => line.startsWith("data:"))
    .map((line) => line.slice(5).trimStart())
    .join("\n");
  return data ? (JSON.parse(data) as T) : undefined;
}

function authHeaders(accept: string = jsonHeaders.Accept): HeadersInit {
  const token = new URLSearchParams(window.location.search).get("token");
  const headers: Record<string, string> = { Accept: accept };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  return headers;
}

/**
 * A frame from `/api/live/stream`. The backend puts both shapes on one stream
 * because cursor expiry is a property of the cursor you connected with, not a
 * separate resource.
 */
export type WebLiveFrame = WebLiveSnapshot | WebLiveCursorExpired;

export function isCursorExpired(frame: WebLiveFrame): frame is WebLiveCursorExpired {
  return (frame as WebLiveCursorExpired).cursorExpired === true;
}

export const apiClient = {
  status: () => getJson<ApiStatus>("/api/status"),
  config: () => getJson<EffectiveConfigSummary>("/api/config/effective"),
  policy: () => getJson<EffectivePolicySummary>("/api/policy/effective"),
  liveSnapshot: () => getJson<WebLiveSnapshot>("/api/live/snapshot"),
  // fetch-based rather than EventSource: EventSource cannot set an
  // Authorization header, and the server rejects query-string tokens, so it
  // would work on loopback and 401 on any other bind.
  liveStream: (
    after: number | undefined,
    onFrame: (frame: WebLiveFrame) => void,
    signal: AbortSignal,
  ) =>
    streamSseJson<WebLiveFrame>(
      after === undefined ? "/api/live/stream" : `/api/live/stream?after=${after}`,
      onFrame,
      signal,
    ),
  agentsLive: () => getJson<WebAgentActivitySnapshot>("/api/agents/live"),
  // The board is rows in the memory database rather than an in-process
  // registry, so unlike `agentsLive` these answer in a standalone `archon web`
  // as well as an attached one.
  boardRuns: () => getJson<WebBoardRunList>("/api/board/runs"),
  boardItems: (runId: string, statuses: string[]) =>
    getJson<WebBoardItems>(
      `/api/board/runs/${encodeURIComponent(runId)}/items${
        statuses.length ? `?status=${encodeURIComponent(statuses.join(","))}` : ""
      }`,
    ),
  boardItemHistory: (itemId: string) =>
    getJson<WebBoardHistory>(
      `/api/board/items/${encodeURIComponent(itemId)}/history`,
    ),
  // Run-scoped and server-capped, so the caller takes what it is given rather
  // than paging: the feed is only ever read from its recent end.
  boardRunActivity: (runId: string) =>
    getJson<WebBoardActivity>(
      `/api/board/runs/${encodeURIComponent(runId)}/activity`,
    ),
  authSession: () => getJson<WebAuthSession>("/api/auth/session"),
  uploadPolicy: () => getJson<WebUploadPolicy>("/api/uploads/policy"),
  uploadIntent: (request: WebUploadIntent) =>
    postJson<WebUploadIntentResponse>("/api/uploads/intent", request),
  chatHistory: () => getJson<WebChatHistoryResponse>("/api/chat/history"),
  submitChat: (request: WebChatSubmitRequest) =>
    postJson<WebChatSubmitResponse>("/api/chat/submit", request, 300_000),
  corpusSummary: () => getJson<CorpusSummary>("/api/corpus/summary"),
  corpusSearch: (query: string, kind: string) =>
    getJson<CorpusSearchResponse>(
      `/api/corpus/search?query=${encodeURIComponent(query)}&kind=${encodeURIComponent(kind)}&limit=80`,
    ),
  corpusSourcePreview: (path: string) =>
    getJson<CorpusSourcePreview>(
      `/api/corpus/source?path=${encodeURIComponent(path)}`,
    ),
  // Binary sources are their own endpoint: `CorpusSourcePreview.content` is a
  // string, and base64 in the preview JSON would inflate every document by a
  // third and park it in the query cache.
  corpusSourceBytes: (path: string) =>
    getBytes(
      `/api/corpus/source/bytes?path=${encodeURIComponent(path)}`,
      "application/pdf",
    ),
  ingestSummary: () => getJson<WebIngestSummary>("/api/ingest/summary"),
  startIngest: (request: WebIngestRunRequest) =>
    postJson<WebIngestRunResponse>("/api/ingest/run", request),
  createKnowledgeBase: (request: WebKbCreateRequest) =>
    postJson<WebKbCreateResponse>("/api/ingest/kb", request),
  deleteDocument: (request: WebDocDeleteRequest) =>
    postJson<WebDocDeleteResponse>("/api/docs/delete", request),
  indexControl: (request: WebIndexControlRequest) =>
    postJson<WebIndexControlResponse>("/api/index/control", request),
  learningSummary: () => getJson<LearningSummary>("/api/learning/summary"),
  cognitiveSummary: () => getJson<CognitiveWebSummary>("/api/cognitive/summary"),
  worldSummary: () => getJson<WorldInspectionSummary>("/api/world/summary"),
  pipelineSummary: () => getJson<PipelineSummary>("/api/pipelines/summary"),
  workflowSummary: () => getJson<WorkflowWebSummary>("/api/workflows/summary"),
  workflowDetail: (runId: string) =>
    getJson<WorkflowRunDetail>(`/api/workflows/${encodeURIComponent(runId)}`),
  workflowEvents: (runId: string, after = 0) =>
    getJson<WorkflowEventPreview[]>(
      `/api/workflows/${encodeURIComponent(runId)}/events?after=${after}`,
    ),
  workflowEventStream: (
    runId: string,
    after: number,
    onEvents: (events: WorkflowEventPreview[]) => void,
    signal: AbortSignal,
  ) =>
    streamSseJson<WorkflowEventPreview[]>(
      `/api/workflows/${encodeURIComponent(runId)}/stream?after=${after}`,
      onEvents,
      signal,
    ),
  workflowControl: (request: WorkflowControlRequest) =>
    postJson<WorkflowControlResponse>("/api/workflows/control", request),
  metricsSummary: () => getJson<MetricsSummary>("/api/metrics/summary"),
  evidenceGraph: () => getJson<EvidenceGraphSummary>("/api/evidence/graph"),
  settingsSummary: () => getJson<SettingsSummary>("/api/settings/summary"),
  themeProfile: () => getJson<WebThemeProfileEnvelope>("/api/settings/theme-profile"),
  saveThemeProfile: (request: WebThemeProfileSaveRequest) =>
    postJson<WebThemeProfileEnvelope>("/api/settings/theme-profile", request),
  evaluateAction: (request: WebActionRequest) =>
    postJson<WebActionResponse>("/api/actions/evaluate", request),
};
