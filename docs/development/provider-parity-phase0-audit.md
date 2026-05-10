# Provider Parity Phase 0 Audit

Source PRD:
`/home/unixdude/Archon-projects/archon/prds/finalisation/PRD-ARCHON-FINALISATION-002-provider-parity-production-polish.md`

## Status

Phase 0 originally proved the current state before capability flags changed.
This audit now records the post-parity state and the remaining honest limits:

- Anthropic is the fully agentic provider.
- Codex is wired for OAuth, spoof headers, one-shot chat, streaming TUI session,
  tool use, subagents, `/btw` side questions, provider-neutral pipelines,
  provider diagnostics, and kill switch behavior.
- Completion integrity is provider-neutral today: the current verifier inspects
  persisted evidence/trust state rather than calling a live model.

The remaining direct Anthropic construction sites are either low-level provider
factories, identity refresh helpers, one-shot legacy helpers, or SDK-specific
surfaces called out below.

## Source Of Truth

| Concern | Source |
|---|---|
| Codex current capability flags | `crates/archon-llm/src/providers/capabilities.rs` |
| Generated capability docs | `docs/generated/provider-capabilities.md` |
| Codex chat/TUI provider | `crates/archon-llm/src/providers/codex/client.rs` |
| Codex tool schema translation | `crates/archon-llm/src/providers/codex/translator/tools.rs` |
| Codex SSE/tool-call event parsing | `crates/archon-llm/src/providers/codex/translator/stream.rs` |
| Existing low-level provider trait | `crates/archon-llm/src/provider.rs` |
| Pipeline LLM trait | `crates/archon-pipeline/src/runner.rs` |
| Pipeline Anthropic adapter | `crates/archon-pipeline/src/llm_adapter.rs` |
| CLI provider gates | `src/command/provider_gate.rs` |

## Current Codex Capability Gap

| Capability | Anthropic status | Codex status today | Required parity work |
|---|---|---|---|
| One-shot chat | supported | supported | Keep existing `LlmProvider` path working. |
| Interactive TUI session | supported | supported | Keep Codex session routing and metadata visible. |
| Streaming | supported | supported | Preserve Codex SSE translation and bounded TUI delivery. |
| Tool use | supported | supported | Codex Responses schema conversion, streamed arguments, and tool-result continuation are pinned by adapter tests. |
| Subagents | supported | supported | `AgentTool`, `SubagentExecutor`, and `SubagentRunner` use `Arc<dyn LlmProvider>`; `codex_subagent_provider_parity` proves the Codex-named loop. |
| Coding pipeline | supported | supported | CLI/TUI coding pipelines use `ProviderLlmAdapter` with active provider model normalization. |
| Research pipeline | supported | supported | CLI/TUI research pipelines use the same provider-neutral adapter. |
| Gametheory pipeline | supported | supported | Classification, specialists, replay, and resume build the active provider through the shared runtime helper. |
| `/btw` | supported | supported | Side questions now route through the active session provider rather than constructing a separate Anthropic client. |
| Completion verification | provider-neutral today | provider-neutral today | Current verifier reads source-of-truth evidence and trust state without constructing a provider client. |
| Vision | supported | supported by matrix | Keep Codex vision docs honest until real image flow is proven. |
| Cost metadata | supported | not capability-enabled | Add provider-neutral usage/cost representation; mark missing exact cost honestly if Codex omits it. |

## Direct Anthropic Construction Baseline

These production paths currently construct `AnthropicClient` directly. They are
the Phase 1/2 refactor targets or approved low-level helpers.

| Path | Count | Current purpose | Parity action |
|---|---:|---|---|
| `src/session_loop/slash_handlers.rs` | 1 | Refresh Anthropic identity headers. | Keep low-level Anthropic-only only if scoped to `/refresh-identity`; otherwise route through provider diagnostics. |
| `src/session.rs` | 2 | Main session and session restore. | Replace remaining low-level construction with provider router where practical. |
| `src/runtime/llm.rs` | 1 | Runtime provider wrapper (one site wrapped in ObservedLlmProvider by PRD-006). | Approved low-level factory; command/pipeline/gametheory callers should use it instead of direct construction. |
| `src/command/chat.rs` | 1 | One-shot Anthropic chat helper. | Route chat through the same provider router used by Codex. |
| `crates/archon-sdk/src/query.rs` | 1 | SDK query client. | Decide whether SDK remains Anthropic-specific or accepts provider selection. |

Guard test:
`tests/finalisation_provider_parity_phase0.rs::phase0_direct_anthropic_construction_baseline_is_explicit`

## Existing Useful Codex Pieces

Codex is not starting from zero. These pieces should be reused rather than
rewritten:

- `CodexProvider::build_request_body()` already converts `LlmRequest` into a
  Responses request.
- `tools_to_responses_tools()` already maps Anthropic-style `name`,
  `description`, `input_schema` tool definitions into Codex function tools.
- `StreamAccumulator` already translates Codex function-call streamed arguments
  into Archon `StreamEvent::ContentBlockStart`, `InputJsonDelta`, and
  `ContentBlockStop`.
- `ResponseInputItem::FunctionCallOutput` already exists, which is the likely
  continuation primitive for sending tool results back to Codex.

The parity proof is not "can Codex ever stream tool calls"; the proof is the
full Archon agentic loop that:

1. receives Codex tool calls,
2. executes Archon tools,
3. appends provider-correct tool results,
4. continues the Codex turn,
5. persists provider/model/activity metadata,
6. handles cancellation, backgrounding, malformed arguments, and cost data.

## Phase 1 Recommended Seam

Do not make pipelines depend directly on `AnthropicClient` or `CodexProvider`.
Introduce a provider-neutral agentic contract above `LlmProvider` and below
session/subagent/pipeline code.

The existing `LlmProvider` is now the practical low-level streaming contract for
session, subagent, team and pipeline paths. The richer `agentic` module remains
the seam for explicit turn-level tests and future cost/cancellation expansion:

- tool result continuation as a first-class operation,
- provider/model metadata on each turn,
- cancellation/budget context,
- provider warnings and capability errors,
- stable tool-call IDs across continuation turns.

## Full-State Verification Template

Trigger event:
`[llm].provider = "openai-codex"` and a command requests an agentic surface.

Process X:
Provider router resolves Codex and checks the requested capability.

Expected outcome Y:
Unsupported provider/surface combinations fail before any provider request is
sent. Supported Codex agentic surfaces complete through the active provider and
carry provider metadata into activity events where those events are emitted.

Source of truth:

- provider capability matrix
- activity events
- pipeline/subagent/gametheory persisted rows
- generated provider docs

Separate read operation:

- `archon providers capabilities`
- `archon providers doctor`
- command-specific inspect/status command
- activity timeline inspection

Boundary cases:

- missing Codex credentials
- `ARCHON_CODEX_DISABLED=1`
- malformed Codex tool arguments
- Codex omits exact usage/cost metadata
