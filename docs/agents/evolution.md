# Governed Agent Evolution

Agent evolution is evidence backed and stored in Cozo. It should not create
markdown memory files.

## Workflow

Generate proposals and memory candidates from the persisted ledger:

```bash
archon agent evolve generate --agent reviewer
archon agent evolve list --agent reviewer
archon agent evolve inspect agent-evo-prop-123
archon agent evolve memory-candidates --agent reviewer
```

Use read-only summaries while reviewing:

```bash
archon agent evolve status --agent reviewer
archon agent evolve history --agent reviewer
archon agent evolve report --agent reviewer
```

Run a ledger-backed shadow evaluation before applying risky changes:

```bash
archon agent evolve shadow agent-evo-prop-123 --task-set regression-suite
```

Approval and apply are separate:

```bash
archon agent evolve approve agent-evo-prop-123
archon agent evolve apply agent-evo-prop-123 --activate
```

Rollback creates a new profile version from an older version:

```bash
archon agent evolve rollback --agent reviewer agent-profile-123 --activate
```

When active profile overlays are enabled, runtime agent metadata is hydrated
from the Cozo performance ledger. `invocation_count`, `quality.applied_rate`,
and `quality.completion_rate` reflect observed ledger rows in memory for the
loaded agent; Archon does not rewrite `meta.json` or create memory files for
this.

## Memory Promotion

Memory candidates are promoted explicitly into the Archon memory graph:

```bash
archon agent evolve memory-promote memory-promotion-123 --dry-run
archon agent evolve memory-promote memory-promotion-123 --min-score 0.85
```

Promotion stores a `Correction` memory in the Cozo-backed memory graph with
agent, candidate, and evidence tags. It does not write `MEMORY.md` or other
memory files.

## Knowledge Digest

Compile a structured digest from governed Cozo evidence:

```bash
archon agent evolve digest --agent reviewer
archon agent evolve digest --agent reviewer --persist
```

Digest claims are stored as typed `AgentKnowledgeClaim` learning events when
`--persist` is used. They remain structured Cozo evidence for reporting and
pipeline context; they are not Markdown memory files and are not treated as
durable truth without their underlying evidence IDs.

## Guardrails

Permission changes remain governed proposals. Parent session mode, sandbox
policy, subagent deny lists, and dangerous-bypass guards stay authoritative.
Evolved profiles may narrow behavior automatically, but risky permission or
provider identity changes require explicit approval.
