# One-Shot Context Handling

Auto-compaction is for growing conversations: main chat sessions, subagents,
and pipelines that repeatedly add turns to the same prompt state.

Bounded one-shot calls do not need auto-compaction because they build a fresh,
isolated request and do not carry forward unbounded history. Examples include
memory extraction, world-model label generation, KB compile completions, and
single-shot `archon chat` calls.

Those calls still rely on provider context-window classification. If a one-shot
request is too large, the provider error should surface as
`ContextWindowExceeded` or a user-visible overflow error. They should not create
adapter-local compaction state or silently summarize unrelated context.
