export function actionResponse(request: { actionId: string; actionKind: string; dryRun: boolean }) {
  const requiresConfirmation = !request.actionKind.startsWith("upload.");
  const policyReason = request.dryRun
    ? `dry-run evaluated for ${request.actionKind}`
    : `action evaluated for ${request.actionKind}`;
  return {
    decision: {
      allowed: request.dryRun,
      requiresConfirmation,
      policyReason,
      dryRunAvailable: true,
    },
    audit: {
      actionId: request.actionId,
      actionKind: request.actionKind,
      allowed: request.dryRun,
      dryRun: request.dryRun,
      policyReason,
      createdAtMs: 1770000001,
    },
  };
}

export function themeProfile(saved?: {
  themeMode: string;
  densityMode: string;
  accentId: string;
  accentHex: string;
  accentStrongHex: string;
  updatedAtMs: number;
}) {
  const profile = saved ?? {
    themeMode: "dark",
    densityMode: "comfortable",
    accentId: "mint",
    accentHex: "#87d8b4",
    accentStrongHex: "#2fbc86",
    updatedAtMs: 1770000000,
  };
  return {
    profile,
    storagePath: "~/.archon/web/theme-profile.json",
    persisted: true,
    exportJson: JSON.stringify(profile, null, 2),
  };
}

export function corpusPreview(path: string) {
  if (path.includes("006A")) {
    return {
      source: corpusSource("World model PRD", "/repo/prds/006A.md", "md", 88000),
      content: "# World Model PRD\n\nLatent next-state prediction, advisor posture, and evidence graph integration.",
      lineCount: 3,
      truncated: false,
      previewAvailable: true,
      policyReason: "read-only preview under configured corpus root",
    };
  }
  return {
    source: corpusSource("README.md", "/repo/README.md", "md", 32000),
    content: "# Archon\n\nLocal-first agentic workbench with evidence, memory, pipelines, and governed learning.",
    lineCount: 3,
    truncated: false,
    previewAvailable: true,
    policyReason: "read-only preview under configured corpus root",
  };
}

export function metricsSummary() {
  return {
    logs: [probe("Logs", "~/.archon/logs", true, 6, 15000)],
    budgets: [probe("Budget", "~/.archon/budget", true, 1, 700)],
    webBundleFiles: 4,
    webBundleBytes: 276000,
    stores: [
      metricStore("sessions", "ready", "~/.archon/sessions", 18, 40000),
      metricStore("world model", "ready", "~/.archon/world-model", 8, 120000),
      metricStore("reasoning quality", "missing", "~/.archon/reasoning-quality", 0, 0),
    ],
    performance: [
      metricValue("Initial JS budget", "276 KB", "< 1.5 MB gzip", "good"),
      metricValue("Tab switch target", "150", "ms", "tracked"),
      metricValue("Live event target", "250", "ms", "tracked"),
    ],
    queues: [
      metricValue("web action audit ledger", "4", "rows", "active"),
      metricValue("reasoning-quality event ledger", "0", "rows", "quiet"),
      metricValue("world advisor event ledger", "2", "rows", "active"),
    ],
    recentEvents: [
      metricEvent("web", "pipeline.pause action evaluated in dry-run mode", "info", "ledger tail"),
      metricEvent("world", "advisor unavailable: cold_start", "warn", "ledger tail"),
    ],
    providerMetrics: [
      providerMetric("anthropic", 8, 1, 2, 42000, 9800, 0.64, 1240, "warn"),
      providerMetric("codex", 5, 0, 0, 23000, 6100, 0, 0, "ok"),
    ],
    providerEvents: [
      providerEvent("anthropic", "claude-sonnet-4-6", "request_failed", "warn", "provider retry recorded before fallback"),
      providerEvent("codex", "gpt-5.4", "request_succeeded", "info", "usage counts recorded"),
    ],
  };
}

export function probe(label: string, path: string, exists: boolean, files: number, bytes: number) {
  return { label, path, exists, files, bytes };
}

export function corpusSource(label: string, path: string, kind: string, bytes: number, excerpt: string | null = null) {
  return { label, path, kind, bytes, excerpt, score: excerpt ? 2.4 : 0, matchKind: excerpt ? "chunk" : "source" };
}

export function corpusChunk(sourceLabel: string, sourcePath: string, chunkLabel: string, lineStart: number, score: number, excerpt: string, embeddingStatus: string) {
  return { sourceLabel, sourcePath, chunkLabel, lineStart, score, excerpt, embeddingStatus };
}

export function docItem(documentId: string, sourcePath: string, mediaType: string, status: string, chunks: number, pages: number, artifacts: number, ocrRuns: number) {
  return { documentId, sourcePath, mediaType, status, chunks, pages, artifacts, ocrRuns, discoveredAt: "2026-05-24T08:00:00Z" };
}

export function videoItem(videoId: string, documentId: string, title: string, source: string, status: string, durationMs: number, chunks: number, transcriptSegments: number, frames: number) {
  return { videoId, documentId, title, source, status, durationMs, chunks, transcriptSegments, frames };
}

export function kbItem(name: string, scope: string, path: string, files: number, bytes: number) {
  return { name, scope, path, files, bytes, exists: true };
}

export function ingestJob(jobId: string, label: string, target: string, command: string, status: string) {
  return {
    jobId,
    label,
    target,
    command,
    status,
    startedAtMs: 1770000000,
    finishedAtMs: status === "running" ? null : 1770000020,
    exitCode: status === "failed" ? 1 : 0,
    stdoutTail: "Ingested source and indexed chunks.",
    stderrTail: "",
  };
}

export function learningSignal(label: string, kind: string, status: string, count: number, path: string) {
  return { label, kind, status, count, path };
}

export function learningRow(label: string, kind: string, status: string, detail: string, path: string) {
  return { label, kind, status, detail, path };
}

export function worldArtifact(label: string, kind: string, status: string, path: string, files: number, bytes: number) {
  return { label, kind, status, path, files, bytes };
}

export function advisorEvent(surface: string, reason: string, actionSummary: string, sessionId: string, createdAt: string) {
  return { surface, reason, actionSummary, sessionId, createdAt };
}

export function worldSignal(label: string, status: string, detail: string) {
  return { label, status, detail };
}

export function worldRow(label: string, kind: string, status: string, detail: string, path: string) {
  return { label, kind, status, detail, path };
}

export function worldPrediction(label: string, surface: string, status: string, detail: string, sessionId: string) {
  return { label, surface, status, detail, sessionId };
}

export function pipelineStage(label: string, family: string, status: string, agentCount: number) {
  return { label, family, status, agentCount };
}

export function pipelineAgent(name: string, family: string, responsibility: string, path: string, status: string) {
  return { name, family, responsibility, path, status };
}

export function pipelineRun(runId: string, family: string, status: string, path: string, updatedAt: string) {
  return { runId, family, status, path, updatedAt };
}

export function pipelineOutput(label: string, kind: string, path: string, bytes: number, updatedAt: string, tail: string) {
  return { label, kind, path, bytes, updatedAt, tail };
}

export function pipelineEvent(sessionId: string, eventType: string, status: string, summary: string, path: string) {
  return { sessionId, eventType, status, summary, path };
}

function metricStore(label: string, status: string, path: string, files: number, bytes: number) {
  return { label, status, path, files, bytes };
}

function metricValue(label: string, value: string, unit: string, status: string) {
  return { label, value, unit, status };
}

function metricEvent(source: string, summary: string, severity: string, createdAt: string) {
  return { source, summary, severity, createdAt };
}

function providerMetric(providerId: string, requestCount: number, errorCount: number, retryCount: number, inputTokens: number, outputTokens: number, estimatedCostUsd: number, latencyMsP95: number, status: string) {
  return { providerId, requestCount, errorCount, retryCount, inputTokens, outputTokens, estimatedCostUsd, latencyMsP95, status };
}

function providerEvent(providerId: string, modelId: string, eventType: string, severity: string, message: string) {
  return { providerId, modelId, eventType, severity, message, createdAt: "2026-05-12T09:10:11Z" };
}
