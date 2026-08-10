export type ApiStatus = { status: string, version: string, web: WebRuntimeStatus, features: WebFeatureSummary, stores: Array<WebStoreStatus>, };

export type WebRuntimeStatus = { bindAddress: string, port: number, authRequired: boolean, maxBodyBytes: number, devMode: boolean, assetMode: string, };

export type WebFeatureSummary = { chat: boolean, uploads: boolean, memoryLearning: boolean, worldModel: boolean, reasoningQuality: boolean, corpus: boolean, pipelines: boolean, workflows: boolean, metrics: boolean, 
/**
 * Whether the terminal WebSocket route was registered at all. The frontend
 * hides the nav entry when this is false, because the route does not exist
 * and every connection attempt would 404.
 */
terminal: boolean, };

export type WebStoreStatus = { name: string, status: string, detail: string, };

export type EffectiveConfigSummary = { web: WebConfigSummary, frontendStack: FrontendStackSummary, };

export type WebConfigSummary = { port: number, bindAddress: string, openBrowser: boolean, maxBodyBytes: number, authRequired: boolean, nonLoopbackBind: boolean, };

export type FrontendStackSummary = { framework: string, bundler: string, generatedTypes: boolean, assetDelivery: string, };

export type EffectivePolicySummary = { web: WebPolicySummary, subsystem: WebSubsystemPolicySummary, actionGate: string, requiresConfirmation: Array<string>, };

export type WebPolicySummary = { allowMutatingActions: boolean, allowFileUploads: boolean, allowPipelineControls: boolean, allowModelTrainingActions: boolean, allowCorpusOpenPaths: boolean, 
/**
 * Mirrors `policy.web.allow_web_terminal`. Its own flag rather than a
 * member of the mutation family: the terminal is arbitrary code execution,
 * not a named action, so no other gate may imply it.
 */
allowWebTerminal: boolean, 
/**
 * Mirrors `policy.web.allow_document_deletion`. Adding a document and
 * permanently destroying one are opposite risks, so enabling uploads does
 * not enable this.
 */
allowDocumentDeletion: boolean, };

export type WebSubsystemPolicySummary = { allowBehaviorProposalActions: boolean, allowModelBehaviorChanges: boolean, allowPipelineControls: boolean, allowCorpusOpenPaths: boolean, allowFileUploads: boolean, };

export type WebActionDecision = { allowed: boolean, requiresConfirmation: boolean, policyReason: string, dryRunAvailable: boolean, };

export type WebLiveEvent = { cursor: number, eventType: string, summary: string, createdAtMs: number, };

export type WebLiveSnapshot = { events: Array<WebLiveEvent>, nextCursor: number, compacted: boolean, };

export type WebLiveCursorExpired = { cursorExpired: boolean, oldestAvailableCursor: number, recovery: string, };

export type WebAgentActivity = { id: string, 
/**
 * Which registry this came from: `background` or `task`.
 */
kind: string, 
/**
 * Human label — the task description, or the runtime subagent id for
 * background agents, whose registry carries no description. Pipeline
 * agents read as `{session}-{ordinal}-{agent}`, which is legible; the
 * other spawn paths mint UUIDs, which are not.
 */
label: string, status: string, elapsedMs: number, };

export type WebAgentActivitySnapshot = { agents: Array<WebAgentActivity>, observedAtMs: number, 
/**
 * `false` when the server was started standalone, where both registries
 * are empty by construction rather than because nothing is running.
 */
attached: boolean, };

export type WebBoardRunList = { 
/**
 * Most recently touched first.
 */
runs: Array<WebBoardRun>, 
/**
 * `false` when there is no memory database yet, which is a different
 * answer from a database holding an empty board and reads differently in
 * the UI.
 */
storeAvailable: boolean, observedAtMs: number, };

export type WebBoardRun = { runId: string, total: number, 
/**
 * Per-status counts, only for statuses the run actually has items in.
 */
counts: Array<WebBoardStatusCount>, 
/**
 * RFC 3339. The newest `updated_at` in the run, and the sort key.
 */
lastUpdatedAt: string, };

export type WebBoardStatusCount = { status: string, count: number, };

export type WebBoardItems = { runId: string, 
/**
 * Oldest first — the board is a queue, and the item raised first has been
 * waiting longest.
 */
items: Array<WebBoardItem>, 
/**
 * The statuses that were filtered on, empty when all were requested.
 */
statuses: Array<string>, storeAvailable: boolean, observedAtMs: number, };

export type WebBoardItem = { id: string, runId: string, 
/**
 * `issue` or `note`.
 */
kind: string, status: string, title: string, 
/**
 * File references and what was observed. Required at the store, so never
 * empty on an item that exists.
 */
evidence: string, 
/**
 * What "done" means for this item.
 */
acceptance: string, raisedBy: string, 
/**
 * The agent currently holding the item, `null` when unclaimed.
 */
claimedBy: string | null, 
/**
 * Attempt counter, 0-based.
 */
round: number, createdAt: string, updatedAt: string, 
/**
 * Why the item was declined. Non-null only on a declined item, and the
 * store refuses to record a decline without one.
 */
declineReason: string | null, };

export type WebBoardHistory = { itemId: string, 
/**
 * Oldest first. Empty for an item that has never transitioned — claims and
 * releases are not recorded, only decisions about the work.
 */
events: Array<WebBoardEvent>, storeAvailable: boolean, };

export type WebBoardEvent = { itemId: string, 
/**
 * Per-item, 0-based.
 */
seq: number, at: string, fromStatus: string, toStatus: string, round: number, 
/**
 * Who held the item at the time, `null` when nobody did.
 */
actor: string | null, 
/**
 * What the transition recorded. Required for a decline, empty otherwise.
 */
note: string, };

export type WebBoardActivity = { runId: string, 
/**
 * Newest first — the opposite of `WebBoardHistory`, which is one item's
 * ladder read from the bottom. A feed is read from the top.
 */
events: Array<WebBoardEvent>, 
/**
 * The server-side cap, never exceeded by `events`.
 */
limit: number, 
/**
 * `true` when older transitions exist beyond `events`.
 */
truncated: boolean, 
/**
 * `false` when there is no memory database yet — a different answer from a
 * database whose board has no history, and read differently in the UI.
 */
storeAvailable: boolean, observedAtMs: number, };

export type WebActionRequest = { actionId: string, actionKind: string, dryRun: boolean, payloadSummary: string, confirmationToken: string | null, };

export type WebActionAuditRow = { actionId: string, actionKind: string, allowed: boolean, dryRun: boolean, policyReason: string, createdAtMs: number, };

export type WebActionResponse = { decision: WebActionDecision, audit: WebActionAuditRow, };

export type WebAuthSession = { authenticated: boolean, authRequired: boolean, transport: string, cookieMode: boolean, csrfRequired: boolean, serverSideLogoutSupported: boolean, logoutMessage: string, };

export type WebChatAttachment = { fileName: string, sizeBytes: number, mimeType: string, accepted: boolean, policyReason: string, dataBase64: string | null, storedPath: string | null, 
/**
 * Docs store id the ingest assigned to this attachment. `null` when the
 * file was never ingested — rejected by policy, or the ingest failed.
 * Absent id and empty id are different facts, so this is never `""`.
 */
documentId: string | null, };

export type WebChatSubmitRequest = { message: string, attachments: Array<WebChatAttachment>, };

export type WebChatSubmitResponse = { messageId: string, accepted: boolean, createdAtMs: number, policyReason: string, storedPath: string, reply: string, attachments: Array<WebChatAttachment>, };

export type WebChatHistoryMessage = { id: string, role: string, title: string, body: string, attachments: Array<WebChatAttachment>, createdAtMs: number, policyReason: string, storedPath: string, };

export type WebChatHistoryResponse = { messages: Array<WebChatHistoryMessage>, storedPath: string, truncated: boolean, };

export type WebUploadPolicy = { enabled: boolean, maxFiles: number, maxBytesPerFile: number, acceptedMimeTypes: Array<string>, policyReason: string, };

export type WebUploadIntent = { fileName: string, sizeBytes: number, mimeType: string, };

export type WebUploadIntentResponse = { decision: WebActionDecision, accepted: boolean, };

export type WebUploadedFile = { 
/**
 * Absolute path on the machine running the server. This is the point of
 * the endpoint: it is what gets typed into the TUI.
 */
path: string, fileName: string, sizeBytes: number, };

export type WebUploadResponse = { accepted: boolean, policyReason: string, files: Array<WebUploadedFile>, };

export type WebDocDeleteRequest = { documentId: string, 
/**
 * The browser has shown the operator what will be removed and they said
 * yes. Without it the request is a dry run that reports the counts.
 */
confirmed: boolean, };

export type WebDocDeleteResponse = { accepted: boolean, policyReason: string, documentId: string, sourcePath: string, chunks: number, pages: number, artifacts: number, vectors: number, };

export type WebIndexControlRequest = { 
/**
 * `pause`, `resume`, `cancel` or `retryFailed`.
 */
action: string, 
/**
 * Required by every action except `retryFailed`.
 */
jobId: string | null, limit: number | null, };

export type WebIndexControlResponse = { accepted: boolean, policyReason: string, detail: string, };

export type CognitiveWebSummary = { store: PathProbe, storePresent: boolean, situationCount: number, toolDecisionCount: number, executiveDecisionCount: number, reflectionCount: number, proposalCount: number, applyResultCount: number, selfModelFactCount: number, metricEventCount: number, metricDefinitionVersion: number, evaluationWindowId: string | null, daemon: CognitiveDaemonPreview, latestTick: CognitiveTickPreview | null, decisions: Array<CognitiveRowPreview>, reflections: Array<CognitiveRowPreview>, proposals: Array<CognitiveRowPreview>, metrics: Array<CognitiveRowPreview>, };
export type CognitiveRowPreview = { id: string, label: string, status: string, detail: string, createdAt: string, };
export type CognitiveTickPreview = { tickId: string, proposalsEvaluated: number, proposalsAutoApplied: number, proposalsDenied: number, errorCount: number, durationMs: number, createdAt: string, };
export type CognitiveDaemonPreview = { running: boolean, stale: boolean, stopRequested: boolean, ticksRun: number, pid: number | null, lastHeartbeatAt: string | null, };

export type CorpusSource = { label: string, path: string, kind: string, bytes: number, excerpt: string | null, score: number, matchKind: string, };

export type CorpusSummary = { roots: Array<PathProbe>, sources: Array<CorpusSource>, totalSources: number, degraded: boolean, };

export type CorpusSearchQuery = { query: string | null, kind: string | null, limit: number | null, };

export type CorpusSearchResponse = { query: string, kind: string, totalMatches: number, degraded: boolean, rankingMode: string, chunkMatches: Array<CorpusChunkHit>, results: Array<CorpusSource>, };

export type CorpusChunkHit = { sourceLabel: string, sourcePath: string, chunkLabel: string, lineStart: number, score: number, excerpt: string, embeddingStatus: string, };

export type CorpusPreviewQuery = { path: string, };

export type CorpusPreviewMode = "text" | "pdf" | "unsupported";

export type CorpusSourcePreview = { source: CorpusSource, content: string, lineCount: number, truncated: boolean, previewAvailable: boolean, 
/**
 * Which viewer the workbench should use. `content` is only populated for
 * [`CorpusPreviewMode::Text`]; a PDF is fetched separately from
 * `/api/corpus/source/bytes`.
 */
previewMode: CorpusPreviewMode, policyReason: string, };

export type WebIngestSummary = { allowed: boolean, policyReason: string, stores: Array<PathProbe>, documents: Array<WebDocStoreItem>, videos: Array<WebVideoStoreItem>, knowledgeBases: Array<WebKnowledgeBaseItem>, kbStats: WebKnowledgeStats, jobs: Array<WebIngestJob>, indexQueue: WebIndexQueueSummary, indexJobs: Array<WebIndexJobItem>, indexFailures: Array<WebIndexFailureItem>, warnings: Array<string>, };

export type WebDocStoreItem = { documentId: string, sourcePath: string, mediaType: string, status: string, chunks: number, pages: number, artifacts: number, ocrRuns: number, discoveredAt: string, };

export type WebVideoStoreItem = { videoId: string, documentId: string, title: string, source: string, status: string, durationMs: number, chunks: number, transcriptSegments: number, frames: number, };

export type WebKnowledgeBaseItem = { name: string, scope: string, path: string, files: number, bytes: number, exists: boolean, };

export type WebKnowledgeStats = { chunks: number, claims: number, entities: number, relations: number, contradictions: number, };

export type WebIndexQueueSummary = { pending: number, leased: number, indexed: number, failed: number, };

export type WebIndexJobItem = { jobId: string, status: string, scope: string, provider: string, leased: number, indexed: number, failed: number, skipped: number, startedAt: string, lastError: string, };

export type WebIndexFailureItem = { chunkId: string, documentId: string, attemptCount: number, lastError: string, updatedAt: string, };

export type WebIngestJob = { jobId: string, label: string, target: string, command: string, status: string, startedAtMs: number, finishedAtMs: number | null, exitCode: number | null, stdoutTail: string, stderrTail: string, };

export type WebIngestRunRequest = { target: string, source: string, frames: string | null, asr: string | null, transcript: string | null, vlm: boolean, metadataOnly: boolean, confirmed: boolean, };

export type WebIngestRunResponse = { accepted: boolean, decision: WebActionDecision, job: WebIngestJob | null, };

export type WebKbCreateRequest = { name: string, scope: string, description: string | null, confirmed: boolean, };

export type WebKbCreateResponse = { accepted: boolean, decision: WebActionDecision, knowledgeBase: WebKnowledgeBaseItem | null, };

export type PathProbe = { label: string, path: string, exists: boolean, files: number, bytes: number, };

export type LearningSummary = { stores: Array<PathProbe>, signals: Array<LearningSignalItem>, memories: Array<LearningRowPreview>, learningEvents: Array<LearningRowPreview>, proposals: Array<LearningRowPreview>, trustDeltas: Array<LearningRowPreview>, recentSessions: Array<string>, sessionCount: number, reasoningStorePresent: boolean, };

export type LearningSignalItem = { label: string, kind: string, status: string, count: number, path: string, };

export type LearningRowPreview = { label: string, kind: string, status: string, detail: string, path: string, };

export type SettingsSummary = { themeModes: Array<string>, densityModes: Array<string>, policyEditingEnabled: boolean, directFilesystemOpenEnabled: boolean, };

export type MetricsSummary = { logs: Array<PathProbe>, budgets: Array<PathProbe>, webBundleFiles: number, webBundleBytes: number, stores: Array<MetricStoreHealth>, performance: Array<MetricValue>, queues: Array<MetricValue>, recentEvents: Array<MetricEventPreview>, providerMetrics: Array<ProviderRuntimeMetric>, providerEvents: Array<ProviderRuntimeEventPreview>, };

export type MetricStoreHealth = { label: string, status: string, path: string, files: number, bytes: number, };

export type MetricValue = { label: string, value: string, unit: string, status: string, };

export type MetricEventPreview = { source: string, summary: string, severity: string, createdAt: string, };

export type ProviderRuntimeMetric = { providerId: string, requestCount: number, errorCount: number, retryCount: number, inputTokens: number, outputTokens: number, estimatedCostUsd: number, latencyMsP95: number, status: string, };

export type ProviderRuntimeEventPreview = { providerId: string, modelId: string, eventType: string, severity: string, message: string, createdAt: string, };

export type PipelineSummary = { definitions: Array<PathProbe>, sessionCount: number, recentSessions: Array<string>, artifactRoots: Array<PathProbe>, stages: Array<PipelineStageSummary>, agents: Array<PipelineAgentSummary>, runs: Array<PipelineRunSummary>, outputs: Array<PipelineOutputSummary>, liveEvents: Array<PipelineLiveEventPreview>, };

export type PipelineStageSummary = { label: string, family: string, status: string, agentCount: number, };

export type PipelineAgentSummary = { name: string, family: string, responsibility: string, path: string, status: string, };

export type PipelineRunSummary = { runId: string, family: string, status: string, path: string, updatedAt: string, };

export type PipelineOutputSummary = { label: string, kind: string, path: string, bytes: number, updatedAt: string, tail: string, };

export type PipelineLiveEventPreview = { sessionId: string, eventType: string, status: string, summary: string, path: string, };

export type WorkflowWebSummary = { root: string, runs: Array<WorkflowRunSummary>, events: Array<WorkflowEventPreview>, controls: Array<WorkflowControlPreview>, };

export type WorkflowRunSummary = { id: string, name: string, status: string, stageCount: number, acceptedCount: number, failedCount: number, artifactCount: number, updatedAt: string, };

export type WorkflowEventPreview = { runId: string, seq: number, kind: string, status: string, summary: string, createdAt: string, };

export type WorkflowControlPreview = { action: string, enabled: boolean, policyReason: string, };

export type WorkflowRunDetail = { summary: WorkflowRunSummary, bundle: WorkflowBundleView | null, approval: WorkflowApprovalView | null, harness: string | null, compiledSpec: string | null, stages: Array<WorkflowStageView>, agents: Array<WorkflowAgentView>, v2Results: Array<WorkflowV2ResultView>, v2Branches: Array<WorkflowV2BranchView>, artifacts: Array<WorkflowArtifactView>, events: Array<WorkflowEventPreview>, };

export type WorkflowBundleView = { workflowPath: string, compiledSpecPath: string, workflowHash: string, compiledHash: string, phaseCount: number, maxAgents: number, maxParallelism: number, writeCapableStages: Array<string>, };

export type WorkflowApprovalView = { workflowHash: string, projectRoot: string, workflowName: string, phaseCount: number, maxAgents: number, maxParallelism: number, writeCapableStages: Array<string>, externalRequirements: Array<string>, costWarning: string, rawScriptPath: string, compiledSpecPath: string, decision: string | null, decidedAt: string | null, decidedBy: string | null, };

export type WorkflowAgentView = { stageId: string, itemId: string, status: string, promptPath: string | null, inputHash: string | null, promptHash: string | null, promptCreatedAt: string | null, provider: string | null, model: string | null, tokensIn: number, tokensOut: number, costUsd: number, artifactId: string | null, artifactPath: string | null, resultPreview: string | null, error: string | null, recentPublicToolCalls: Array<WorkflowToolCallPreview>, outputPath: string, };

export type WorkflowToolCallPreview = { toolName: string, inputPreview: string | null, outputPreview: string | null, };

export type WorkflowV2ResultView = { callId: string, status: string, summary: string, resultPath: string, artifactCount: number, branchCount: number, };

export type WorkflowV2BranchView = { callId: string, itemId: string, role: string, status: string, summary: string | null, error: string | null, outputPath: string, };

export type WorkflowStageView = { id: string, status: string, attempt: number, startedAt: string | null, completedAt: string | null, artifacts: number, error: string | null, };

export type WorkflowArtifactView = { id: string, path: string, producingStage: string, contentHash: string, };

export type WorkflowControlRequest = { runId: string, action: string, stageId: string | null, itemId: string | null, rationale: string | null, confirmationToken: string | null, };

export type WorkflowControlResponse = { allowed: boolean, policyReason: string, run: WorkflowRunSummary | null, };

export type WebThemeProfile = { themeMode: string, densityMode: string, accentId: string, accentHex: string, accentStrongHex: string, updatedAtMs: number, };

export type WebThemeProfileEnvelope = { profile: WebThemeProfile, storagePath: string, persisted: boolean, exportJson: string, };

export type WebThemeProfileSaveRequest = { profile: WebThemeProfile, };

export type WorldInspectionSummary = { root: PathProbe, ledgers: Array<PathProbe>, dbPresent: boolean, candidateCount: number, reasoningRootPresent: boolean, artifacts: Array<WorldModelArtifact>, advisorEvents: Array<WorldAdvisorEventPreview>, signals: Array<WorldModelSignal>, candidates: Array<WorldModelRowPreview>, reasoningEvents: Array<WorldModelRowPreview>, shadowReports: Array<WorldModelRowPreview>, predictions: Array<WorldPredictionPreview>, jepa: JepaInspectionSummary, };

export type JepaInspectionSummary = { root: PathProbe, candidateCount: number, evalCount: number, trainingRunCount: number, comparisonCount: number, artifacts: Array<WorldModelArtifact>, signals: Array<WorldModelSignal>, candidates: Array<WorldModelRowPreview>, evals: Array<WorldModelRowPreview>, trainingRuns: Array<WorldModelRowPreview>, comparisons: Array<WorldModelRowPreview>, };

export type WorldModelArtifact = { label: string, kind: string, status: string, path: string, files: number, bytes: number, };

export type WorldAdvisorEventPreview = { surface: string, reason: string, actionSummary: string, sessionId: string, createdAt: string, };

export type WorldModelSignal = { label: string, status: string, detail: string, };

export type WorldModelRowPreview = { label: string, kind: string, status: string, detail: string, path: string, };

export type WorldPredictionPreview = { label: string, surface: string, status: string, detail: string, sessionId: string, };

export type EvidenceGraphNode = { id: string, label: string, kind: string, detail: string, count: number, };

export type EvidenceGraphEdge = { id: string, source: string, target: string, label: string, };

export type EvidenceGraphSummary = { nodeBudget: number, edgeBudget: number, sourceCount: number, relationCount: number, degraded: boolean, nodes: Array<EvidenceGraphNode>, edges: Array<EvidenceGraphEdge>, };
