# Memory System Promotion

Archon does not use Markdown memory files as the durable agent memory backend.
Memory promotion flows through the existing Cozo-backed memory graph and
governed learning events.

## Candidate Review

```bash
archon agent evolve memory-candidates --agent reviewer
archon agent evolve memory-promote memory-promotion-123 --dry-run
archon agent evolve memory-promote memory-promotion-123 --min-score 0.85
```

Candidates are evidence-backed suggestions. Promotion stores structured memory
with tags for agent, candidate, and evidence IDs. It does not write `MEMORY.md`,
daily notes, or dream journals.

## Digest Flow

```bash
archon agent evolve digest --agent reviewer
archon agent evolve digest --agent reviewer --persist
```

Persisted digests become typed learning events. They are useful for reporting
and future context, but they remain claims linked to evidence, not unreviewed
truth.

## Privacy

Promotion should preserve privacy tier and provenance where available. Raw
tokens, provider credentials, sandbox sync roots, and generated memory databases
must not be copied into sandbox backends or provider adapters.
