import type { Page, Route } from "@playwright/test";
import { graphEdges, graphNodes } from "./mockGraph";

export async function mockApi(page: Page) {
  const responses: Record<string, unknown> = {
    "/api/status": {
      status: "ok",
      version: "1.2.5",
      web: {
        bindAddress: "127.0.0.1",
        port: 8421,
        authRequired: false,
        devMode: true,
        assetMode: "vite",
      },
      features: {
        chat: true,
        uploads: true,
        memoryLearning: true,
        worldModel: true,
        reasoningQuality: true,
        corpus: true,
        pipelines: true,
        metrics: true,
      },
      stores: [
        { name: "memory", status: "ready", detail: "~/.archon/memory" },
        { name: "world-model", status: "ready", detail: "~/.archon/world-model" },
      ],
    },
    "/api/config/effective": {
      web: {
        port: 8421,
        bindAddress: "127.0.0.1",
        openBrowser: false,
        authRequired: false,
        nonLoopbackBind: false,
      },
      frontendStack: {
        framework: "React 19",
        bundler: "Vite",
        generatedTypes: true,
        assetDelivery: "embedded",
      },
    },
    "/api/policy/effective": {
      web: {
        allowMutatingActions: false,
        allowFileUploads: true,
        allowPipelineControls: false,
        allowModelTrainingActions: false,
        allowCorpusOpenPaths: false,
      },
      subsystem: {
        allowBehaviorProposalActions: true,
        allowModelBehaviorChanges: false,
        allowPipelineControls: false,
        allowCorpusOpenPaths: false,
        allowFileUploads: true,
      },
      actionGate: "web policy AND subsystem policy",
      requiresConfirmation: ["pipeline controls", "training actions"],
    },
    "/api/live/snapshot": {
      events: [
        {
          cursor: 7,
          eventType: "web.live.snapshot",
          summary: "Snapshot loaded",
          createdAtMs: 1,
        },
      ],
      nextCursor: 8,
      compacted: false,
    },
    "/api/auth/session": {
      authenticated: true,
      authRequired: false,
      transport: "loopback",
      cookieMode: false,
      csrfRequired: false,
    },
    "/api/uploads/policy": {
      enabled: true,
      maxFiles: 5,
      maxBytesPerFile: 10485760,
      acceptedMimeTypes: ["text/plain", "application/pdf"],
      policyReason: "allowed by local policy",
    },
    "/api/uploads/intent": {
      accepted: true,
      decision: {
        allowed: true,
        requiresConfirmation: false,
        policyReason: "upload intent accepted by web upload policy",
        dryRunAvailable: true,
      },
    },
    "/api/chat/submit": {
      messageId: "webmsg_test",
      accepted: true,
      createdAtMs: 1770000000,
      policyReason: "chat message accepted and recorded by the web workbench",
      storedPath: "~/.archon/web/chat.messages.jsonl",
    },
    "/api/corpus/summary": {
      roots: [probe("Repository docs", "/repo/docs", true, 42, 800000)],
      sources: [
        corpusSource("README.md", "/repo/README.md", "md", 32000),
        corpusSource("World model PRD", "/repo/prds/006A.md", "md", 88000),
      ],
      totalSources: 2,
      degraded: false,
    },
    "/api/corpus/search": {
      query: "",
      kind: "",
      totalMatches: 2,
      degraded: false,
      rankingMode: "chunk_lexical_with_embedding_hints",
      chunkMatches: [
        corpusChunk("README.md", "/repo/README.md", "chunk 1", 1, 2.6, "Archon workbench overview", "filesystem"),
        corpusChunk("World model PRD", "/repo/prds/006A.md", "chunk 3", 42, 1.9, "World model design", "indexed-corpus"),
      ],
      results: [
        corpusSource("README.md", "/repo/README.md", "md", 32000, "Archon workbench overview"),
        corpusSource("World model PRD", "/repo/prds/006A.md", "md", 88000, "World model design"),
      ],
    },
    "/api/corpus/source": {
      source: corpusSource("README.md", "/repo/README.md", "md", 32000),
      content: "# Archon\n\nLocal-first agentic workbench with evidence, memory, pipelines, and governed learning.",
      lineCount: 3,
      truncated: false,
      previewAvailable: true,
      policyReason: "read-only preview under configured corpus root",
    },
    "/api/learning/summary": {
      stores: [probe("Self calibration", "~/.archon/self-calibration", true, 12, 9000)],
      signals: [
        learningSignal("session activity", "sessions", "present", 18, "~/.archon/sessions"),
        learningSignal("reasoning quality", "reasoning", "present", 9, "~/.archon/reasoning-quality"),
        learningSignal("self trust", "calibration", "present", 5, "~/.archon/self-calibration/trust/self-trust.json"),
        learningSignal("behaviour proposals", "proposal", "missing", 0, "~/.archon/behaviour"),
      ],
      memories: [
        learningRow("source verification habit", "memory", "recorded", "Read the real source tree before architectural claims.", "~/.archon/memory/garden/rule.json"),
      ],
      learningEvents: [
        learningRow("claim corrected", "learning_event", "recorded", "User correction linked to self-trust update.", "~/.archon/learning/events.jsonl"),
      ],
      proposals: [
        learningRow("proactive briefing", "proposal", "pending", "Surface pending behaviour proposals at session start.", "~/.archon/behaviour/proposals.jsonl"),
      ],
      trustDeltas: [
        learningRow("architecture-advice", "trust", "recorded", "{\"score\":0.72,\"negative\":2}", "~/.archon/self-calibration/trust/self-trust.json"),
      ],
      recentSessions: ["session-a", "session-b"],
      sessionCount: 2,
      reasoningStorePresent: true,
    },
    "/api/world/summary": {
      root: probe("World model", "~/.archon/world-model", true, 8, 120000),
      ledgers: [probe("Trace ledger", "~/.archon/world-model/traces", true, 4, 32000)],
      dbPresent: true,
      candidateCount: 1,
      reasoningRootPresent: true,
      artifacts: [
        worldArtifact("World database", "cozo", "present", "~/.archon/world-model/world-model.db", 1, 96000),
        worldArtifact("Candidate registry", "checkpoint", "present", "~/.archon/world-model/candidates", 2, 18000),
        worldArtifact("Reasoning bridge", "reasoning", "present", "~/.archon/reasoning-quality", 5, 24000),
      ],
      advisorEvents: [
        advisorEvent("pipeline", "cold_start", "advisor returned None until the trace corpus warms", "session-a", "2026-05-12T09:10:11Z"),
        advisorEvent("provider", "available", "prediction attached to fallback ranking", "session-b", "2026-05-12T10:12:13Z"),
      ],
      signals: [
        worldSignal("Cold start gate", "cold_start", "2 cold-start rows in recent advisor ledger"),
        worldSignal("Candidate store", "present", "world-model/candidates"),
        worldSignal("Active pointer", "missing", "world-model/active"),
        worldSignal("Reasoning-quality bridge", "present", "~/.archon/reasoning-quality"),
      ],
      candidates: [
        worldRow("candidate-001.json", "candidate", "recorded", "heldout cosine improved 12%", "~/.archon/world-model/candidates/candidate-001.json"),
      ],
      reasoningEvents: [
        worldRow("claim_before_source_read", "reasoning", "recorded", "high-confidence claim emitted before source read", "~/.archon/reasoning-quality/events.jsonl"),
      ],
      shadowReports: [
        worldRow("shadow-report.json", "shadow", "recorded", "precision gate sample awaiting labels", "~/.archon/reasoning-quality/shadow/report.json"),
      ],
      predictions: [
        worldPrediction("advisor unavailable", "pipeline", "cold_start", "advisor returned None until warm", "session-a"),
        worldPrediction("advisor prediction", "provider", "available", "fallback ranking attached", "session-b"),
      ],
    },
    "/api/pipelines/summary": {
      definitions: [probe("Agents", ".archon/agents", true, 5, 11000)],
      sessionCount: 3,
      recentSessions: ["archon-code-1", "archon-research-2"],
      artifactRoots: [probe("Audit bundles", "~/.archon/pipelines", true, 9, 76000)],
      stages: [
        pipelineStage("Intake", "coding", "ready", 8),
        pipelineStage("Implementation", "coding", "ready", 12),
        pipelineStage("Verification", "coding", "ready", 7),
        pipelineStage("Research", "research", "ready", 10),
        pipelineStage("Game theory", "gametheory", "missing", 0),
      ],
      agents: [
        pipelineAgent("contract agent", "coding", "Parse and lock the task contract", "/repo/.archon/agents/coding-pipeline/contract-agent.md", "ready"),
        pipelineAgent("code generator", "coding", "Implement bounded code changes", "/repo/.archon/agents/coding-pipeline/code-generator.md", "ready"),
        pipelineAgent("final reviewer", "research", "Review stage output and evidence", "/repo/.archon/agents/phdresearch/final-reviewer.md", "ready"),
      ],
      runs: [
        pipelineRun("archon-code-1", "coding", "recorded", "~/.archon/pipelines/archon-code-1", "1770000000"),
        pipelineRun("archon-research-2", "research", "completed", "~/.archon/pipelines/archon-research-2", "1769999900"),
      ],
      outputs: [
        pipelineOutput("report.md", "md", "~/.archon/pipelines/archon-research-2/report.md", 26000, "1769999900", "Final research report written."),
        pipelineOutput("audit.json", "json", "~/.archon/pipelines/archon-code-1/audit.json", 12000, "1770000000", "{\"stage\":\"verification\",\"status\":\"recorded\"}"),
      ],
      liveEvents: [
        pipelineEvent("archon-code-1", "pipeline", "recorded", "pipeline specialist completed implementation verification", "~/.archon/sessions/archon-code-1.activity.jsonl"),
        pipelineEvent("archon-research-2", "activity", "recorded", "research pipeline wrote final report artifact", "~/.archon/sessions/archon-research-2.activity.jsonl"),
      ],
    },
    "/api/metrics/summary": metricsSummary(),
    "/api/evidence/graph": {
      nodeBudget: 3000,
      edgeBudget: 10000,
      sourceCount: 42,
      relationCount: 120,
      degraded: false,
      nodes: graphNodes(),
      edges: graphEdges(),
    },
    "/api/settings/summary": {
      themeModes: ["dark", "light"],
      densityModes: ["compact", "comfortable"],
      policyEditingEnabled: false,
      directFilesystemOpenEnabled: false,
    },
    "/api/settings/theme-profile": themeProfile(),
  };

  await page.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (!url.pathname.startsWith("/api/")) {
      await route.continue();
      return;
    }
    if (url.pathname === "/api/actions/evaluate") {
      const request = route.request().postDataJSON();
      await json(route, actionResponse(request));
      return;
    }
    if (url.pathname === "/api/settings/theme-profile" && route.request().method() === "POST") {
      const request = route.request().postDataJSON();
      await json(route, themeProfile(request.profile));
      return;
    }
    if (url.pathname === "/api/corpus/source") {
      await json(route, corpusPreview(url.searchParams.get("path") ?? ""));
      return;
    }
    const body = responses[url.pathname];
    await json(route, body ?? {}, body === undefined ? 404 : 200);
  });
}

async function json(route: Route, body: unknown, status = 200) {
  await route.fulfill({
    status,
    contentType: "application/json",
    body: JSON.stringify(body),
  });
}

function actionResponse(request: { actionId: string; actionKind: string; dryRun: boolean }) {
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

function themeProfile(saved?: {
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

function corpusPreview(path: string) {
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

function metricsSummary() {
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

function probe(label: string, path: string, exists: boolean, files: number, bytes: number) {
  return { label, path, exists, files, bytes };
}

function corpusSource(label: string, path: string, kind: string, bytes: number, excerpt: string | null = null) {
  return { label, path, kind, bytes, excerpt, score: excerpt ? 2.4 : 0, matchKind: excerpt ? "chunk" : "source" };
}

function corpusChunk(sourceLabel: string, sourcePath: string, chunkLabel: string, lineStart: number, score: number, excerpt: string, embeddingStatus: string) {
  return { sourceLabel, sourcePath, chunkLabel, lineStart, score, excerpt, embeddingStatus };
}

function learningSignal(label: string, kind: string, status: string, count: number, path: string) {
  return { label, kind, status, count, path };
}

function learningRow(label: string, kind: string, status: string, detail: string, path: string) {
  return { label, kind, status, detail, path };
}

function worldArtifact(label: string, kind: string, status: string, path: string, files: number, bytes: number) {
  return { label, kind, status, path, files, bytes };
}

function advisorEvent(surface: string, reason: string, actionSummary: string, sessionId: string, createdAt: string) {
  return { surface, reason, actionSummary, sessionId, createdAt };
}

function worldSignal(label: string, status: string, detail: string) {
  return { label, status, detail };
}

function worldRow(label: string, kind: string, status: string, detail: string, path: string) {
  return { label, kind, status, detail, path };
}

function worldPrediction(label: string, surface: string, status: string, detail: string, sessionId: string) {
  return { label, surface, status, detail, sessionId };
}

function pipelineStage(label: string, family: string, status: string, agentCount: number) {
  return { label, family, status, agentCount };
}

function pipelineAgent(name: string, family: string, responsibility: string, path: string, status: string) {
  return { name, family, responsibility, path, status };
}

function pipelineRun(runId: string, family: string, status: string, path: string, updatedAt: string) {
  return { runId, family, status, path, updatedAt };
}

function pipelineOutput(label: string, kind: string, path: string, bytes: number, updatedAt: string, tail: string) {
  return { label, kind, path, bytes, updatedAt, tail };
}

function pipelineEvent(sessionId: string, eventType: string, status: string, summary: string, path: string) {
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
