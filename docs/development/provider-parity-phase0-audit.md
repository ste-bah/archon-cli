# Provider Parity Phase 0 Audit

Source PRD:
`/home/unixdude/Archon-projects/archon/prds/finalisation/PRD-ARCHON-FINALISATION-002-provider-parity-production-polish.md`

## Status

Phase 0 proves the current state before any capability flags are changed:

- Anthropic is the fully agentic provider.
- Codex is wired for OAuth, spoof headers, one-shot chat, streaming TUI session,
  provider diagnostics, and kill switch behavior.
- Codex is intentionally not yet marked as supporting tools, subagents,
  pipelines, `/btw`, or agentic completion verification.

The next implementation phases must remove the direct Anthropic-only
construction sites by introducing a provider-neutral agentic contract and Codex
tool-result continuation.

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
| Tool use | supported | not capability-enabled | Prove Codex Responses tool schema, streamed arguments, tool-result continuation, and malformed argument handling. |
| Subagents | supported | not capability-enabled | Refactor `AgentTool` and `SubagentExecutor` through provider-neutral agentic turns. |
| Coding pipeline | supported | not capability-enabled | Replace direct `AnthropicLlmAdapter` construction with provider-neutral pipeline adapter. |
| Research pipeline | supported | not capability-enabled | Same as coding, including citation/provenance persistence. |
| Gametheory pipeline | supported | not capability-enabled | Route Tier 1 and specialists through active provider. |
| `/btw` | supported | not capability-enabled | Route side question through active provider and preserve context/tool requirements. |
| Completion verification | supported where LLM-backed | not proven | Route LLM-backed verification through active provider and persist provider/model on incidents. |
| Vision | supported | supported by matrix | Keep Codex vision docs honest until real image flow is proven. |
| Cost metadata | supported | not capability-enabled | Add provider-neutral usage/cost representation; mark missing exact cost honestly if Codex omits it. |

## Direct Anthropic Construction Baseline

These production paths currently construct `AnthropicClient` directly. They are
the Phase 1/2 refactor targets or approved low-level helpers.

| Path | Count | Current purpose | Parity action |
|---|---:|---|---|
| `src/session_loop/slash_handlers.rs` | 1 | Refresh Anthropic identity headers. | Keep low-level Anthropic-only only if scoped to `/refresh-identity`; otherwise route through provider diagnostics. |
| `src/session.rs` | 4 | Main session, session restore, pipeline launch, `/btw`. | Replace agentic turn/pipeline/side-question construction with provider router. |
| `src/command/team.rs` | 1 | Team command LLM client. | Route through provider-neutral agentic contract. |
| `src/runtime/llm.rs` | 1 | Runtime provider wrapper. | May remain as approved low-level factory if all callers use it. |
| `src/command/chat.rs` | 1 | One-shot Anthropic chat helper. | Route chat through the same provider router used by Codex. |
| `src/command/pipeline.rs` | 3 | Coding/research/resume pipeline clients. | Replace with provider-neutral pipeline adapter. |
| `src/command/gametheory.rs` | 1 | Gametheory pipeline client. | Replace with active provider pipeline adapter. |
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

The missing piece is not "can Codex ever stream tool calls"; the missing piece is
a full Archon agentic loop that:

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

The existing `LlmProvider` is a useful low-level streaming abstraction, but it is
not yet a complete agentic contract because it does not model:

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
Until parity lands, unsupported Codex agentic surfaces fail before any provider
request is sent. After parity lands, the same command completes through Codex and
persists provider metadata.

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
