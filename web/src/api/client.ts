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
  WorkflowWebSummary,
  WebKbCreateRequest,
  WebKbCreateResponse,
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

function authHeaders(): HeadersInit {
  const token = new URLSearchParams(window.location.search).get("token");
  return token
    ? { ...jsonHeaders, Authorization: `Bearer ${token}` }
    : jsonHeaders;
}

export const apiClient = {
  status: () => getJson<ApiStatus>("/api/status"),
  config: () => getJson<EffectiveConfigSummary>("/api/config/effective"),
  policy: () => getJson<EffectivePolicySummary>("/api/policy/effective"),
  liveSnapshot: () => getJson<WebLiveSnapshot>("/api/live/snapshot"),
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
  ingestSummary: () => getJson<WebIngestSummary>("/api/ingest/summary"),
  startIngest: (request: WebIngestRunRequest) =>
    postJson<WebIngestRunResponse>("/api/ingest/run", request),
  createKnowledgeBase: (request: WebKbCreateRequest) =>
    postJson<WebKbCreateResponse>("/api/ingest/kb", request),
  learningSummary: () => getJson<LearningSummary>("/api/learning/summary"),
  cognitiveSummary: () => getJson<CognitiveWebSummary>("/api/cognitive/summary"),
  worldSummary: () => getJson<WorldInspectionSummary>("/api/world/summary"),
  pipelineSummary: () => getJson<PipelineSummary>("/api/pipelines/summary"),
  workflowSummary: () => getJson<WorkflowWebSummary>("/api/workflows/summary"),
  metricsSummary: () => getJson<MetricsSummary>("/api/metrics/summary"),
  evidenceGraph: () => getJson<EvidenceGraphSummary>("/api/evidence/graph"),
  settingsSummary: () => getJson<SettingsSummary>("/api/settings/summary"),
  themeProfile: () => getJson<WebThemeProfileEnvelope>("/api/settings/theme-profile"),
  saveThemeProfile: (request: WebThemeProfileSaveRequest) =>
    postJson<WebThemeProfileEnvelope>("/api/settings/theme-profile", request),
  evaluateAction: (request: WebActionRequest) =>
    postJson<WebActionResponse>("/api/actions/evaluate", request),
};
