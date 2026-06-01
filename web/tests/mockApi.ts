import type { Page, Route } from "@playwright/test";
import { appendChatMessages, chatHistory, chatSubmitResponse } from "./mockChat";
import {
  actionResponse,
  advisorEvent,
  corpusChunk,
  corpusPreview,
  corpusSource,
  docItem,
  ingestJob,
  kbItem,
  learningRow,
  learningSignal,
  metricsSummary,
  pipelineAgent,
  pipelineEvent,
  pipelineOutput,
  pipelineRun,
  pipelineStage,
  probe,
  themeProfile,
  videoItem,
  worldArtifact,
  worldPrediction,
  worldRow,
  worldSignal,
} from "./mockFixtures";
import { graphEdges, graphNodes } from "./mockGraph";

export async function mockApi(page: Page) {
  const chatMessages: Array<Record<string, unknown>> = [];
  const responses: Record<string, unknown> = {
    "/api/status": {
      status: "ok",
      version: "1.2.8",
      web: {
        bindAddress: "127.0.0.1",
        port: 8421,
        authRequired: false,
        maxBodyBytes: 67108864,
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
        maxBodyBytes: 67108864,
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
      serverSideLogoutSupported: false,
      logoutMessage: "No bearer token is required for this local web session.",
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
    "/api/ingest/summary": {
      allowed: true,
      policyReason: "web ingest actions allowed by policy",
      stores: [
        probe("document store", "/repo/.archon/archon-data.db", true, 1, 48000),
        probe("project docs", "/repo/.archon/docs", true, 2, 18000),
        probe("project kb", "/repo/.archon/kb", true, 1, 8000),
        probe("video artifacts", "/repo/.archon/video-artifacts", true, 3, 68000),
      ],
      documents: [
        docItem("doc-1", "/repo/docs/architecture.md", "text/markdown", "Processed", 14, 3, 2, 0),
        docItem("doc-2", "/repo/hld/design.pdf", "application/pdf", "Processed", 22, 9, 4, 1),
      ],
      videos: [
        videoItem("video-1", "doc-video-1", "Architecture walkthrough", "https://youtu.be/example", "processed", 300000, 48, 42, 12),
      ],
      knowledgeBases: [
        kbItem("project-evidence", "project", "/repo/.archon/kb/project-evidence", 3, 9000),
      ],
      kbStats: { chunks: 84, claims: 12, entities: 9, relations: 5, contradictions: 1 },
      jobs: [
        ingestJob("job-1", "design.pdf", "docs", "archon docs ingest /repo/hld/design.pdf", "completed"),
      ],
      indexQueue: { pending: 12, leased: 4, indexed: 68, failed: 1 },
      indexJobs: [
        {
          jobId: "idx-1",
          status: "running",
          scope: "pending",
          provider: "fastembed",
          leased: 16,
          indexed: 68,
          failed: 1,
          skipped: 0,
          startedAt: "2026-05-31T10:00:00Z",
          lastError: "",
        },
      ],
      indexFailures: [
        {
          chunkId: "chunk-failed-1",
          documentId: "doc-2",
          attemptCount: 2,
          lastError: "provider timeout",
          updatedAt: "2026-05-31T10:05:00Z",
        },
      ],
      warnings: [],
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
      jepa: {
        root: probe("JEPA", "~/.archon/world-model/jepa", true, 8, 54000),
        candidateCount: 1,
        evalCount: 1,
        trainingRunCount: 1,
        comparisonCount: 1,
        artifacts: [
          worldArtifact("JEPA root", "jepa", "present", "~/.archon/world-model/jepa", 8, 54000),
          worldArtifact("candidate registry", "candidate", "present", "~/.archon/world-model/jepa/candidates", 1, 14000),
        ],
        signals: [
          worldSignal("Candidate registry", "present", "1 candidate files"),
          worldSignal("Latest eval gate", "failed", "candidate-001"),
          worldSignal("Latest training run", "present", "loss 0.047"),
        ],
        candidates: [
          worldRow("candidate-001.json", "candidate", "recorded", "JEPA candidate checkpoint", "~/.archon/world-model/jepa/candidates/candidate-001.json"),
        ],
        evals: [
          worldRow("eval-001.json", "eval", "failed", "rank gate failed", "~/.archon/world-model/jepa/evals/eval-001.json"),
        ],
        trainingRuns: [
          worldRow("train-001.jsonl", "training", "recorded", "MLX Metal training run", "~/.archon/world-model/jepa/training-runs/train-001.jsonl"),
        ],
        comparisons: [
          worldRow("compare-001.json", "comparison", "recorded", "representation comparison", "~/.archon/world-model/jepa/representation-comparisons/compare-001.json"),
        ],
      },
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
    if (url.pathname === "/api/ingest/run") {
      const request = route.request().postDataJSON();
      await json(route, {
        accepted: true,
        decision: {
          allowed: true,
          requiresConfirmation: true,
          policyReason: "ingest accepted by mock policy",
          dryRunAvailable: true,
        },
        job: ingestJob("job-new", request.source || request.target, request.target, `archon ${request.target} ingest`, "running"),
      });
      return;
    }
    if (url.pathname === "/api/ingest/kb") {
      const request = route.request().postDataJSON();
      await json(route, {
        accepted: true,
        decision: {
          allowed: true,
          requiresConfirmation: true,
          policyReason: "kb create accepted by mock policy",
          dryRunAvailable: true,
        },
        knowledgeBase: kbItem(request.name, request.scope, `/repo/.archon/kb/${request.name}`, 1, 1200),
      });
      return;
    }
    if (url.pathname === "/api/settings/theme-profile" && route.request().method() === "POST") {
      const request = route.request().postDataJSON();
      await json(route, themeProfile(request.profile));
      return;
    }
    if (url.pathname === "/api/chat/submit") {
      const request = route.request().postDataJSON();
      const response = chatSubmitResponse(request);
      appendChatMessages(chatMessages, request, response);
      await json(route, response);
      return;
    }
    if (url.pathname === "/api/chat/history") {
      await json(route, chatHistory(chatMessages));
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
