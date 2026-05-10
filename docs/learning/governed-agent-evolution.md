# Governed Agent Evolution

Governed agent evolution is the learning loop that turns runtime evidence into
reviewable proposals, shadow evaluations, profile versions, and rollback paths.

## Durable Stores

Durable evidence is stored in Cozo by default:

- provider runtime events
- provider auth profiles
- permission runtime events
- sandbox sessions and runtime events
- agent performance ledger rows
- evolution proposals
- profile versions
- shadow evaluations
- knowledge claims and memory promotion events

Temporary caches may remain outside Cozo when they are not durable evidence.

## Lifecycle

```text
runtime evidence -> proposal -> review -> shadow evaluation -> apply -> monitor -> rollback
```

High-risk changes require approval. Activation of high-risk, permission-impacting
or provider-identity-impacting changes requires the latest shadow evaluation to
promote with zero regressions.

## Not Allowed

Governed evolution must not:

- write Markdown memory files as canonical memory
- silently grant tools
- override parent session restrictions
- move provider credentials into sandbox backends
- weaken Anthropic Claude Code spoofing
- treat promotion output as truth without evidence IDs
