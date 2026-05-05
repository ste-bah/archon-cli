# Provider capabilities

Generated from `archon_llm::providers::capabilities`. This matrix is Archon surface support, not only raw model wire features.

Archon provider capability matrix

| Provider | Auth mode | one-shot | TUI | stream | tools | subagents | code | research | gametheory | /btw | vision | embed | cost | Notes |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| `anthropic-oauth` | Claude/Anthropic OAuth | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | no | yes | Primary path for agents, subagents, pipelines and /btw. |
| `anthropic-api-key` | ANTHROPIC_API_KEY | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | no | yes | Native Anthropic Messages API path. |
| `anthropic-compatible-proxy` | ANTHROPIC_BASE_URL + API key | yes | yes | yes | yes | yes | yes | yes | yes | no | yes | no | yes | Depends on proxy fidelity; /btw OAuth-only behavior is not assumed. |
| `openai-codex` | ChatGPT/Codex OAuth | yes | yes | yes | no | no | yes | yes | yes | yes | yes | no | no | Backs one-shot chat, full TUI sessions, /btw, and provider-neutral pipelines; subagents are not fully proven yet. |

## Capability keys

- `one_shot_chat` - one-shot chat
- `interactive_session` - interactive TUI
- `streaming` - streaming
- `tool_use` - agent tool use
- `subagents` - subagents
- `pipeline_coding` - coding pipeline
- `pipeline_research` - research pipeline
- `pipeline_gametheory` - gametheory pipeline
- `btw_side_question` - /btw
- `vision` - vision
- `embeddings` - embeddings
- `cost_metadata` - cost metadata
