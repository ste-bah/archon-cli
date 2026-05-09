# Cloud And Local Providers

Bedrock, Vertex, OpenAI, local, and other non-Codex providers share the
provider-neutral runtime contract. Durable runtime evidence goes to Cozo by
default; temporary probe caches may remain outside Cozo.

## Runtime Status

```bash
archon providers status
archon providers status --json
archon providers report --json
```

Status records describe construction readiness, auth profile selection when
available, fallback state, and recent provider failures. Redaction applies
before persistence.

## Cloud Providers

Cloud providers should expose missing-region, missing-project, missing-model,
and missing-credential states as provider runtime status instead of falling
through to Anthropic silently.

## Local Providers

Local providers should report endpoint reachability and selected model without
claiming cloud auth or subscription state. Local provider failures are still
provider runtime evidence when they affect a run.

## Shared Guardrails

No provider may broaden permissions. Tool execution always goes through
permission preflight and sandbox routing.
