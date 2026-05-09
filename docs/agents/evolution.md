# Governed Agent Evolution

Agent evolution is evidence backed and stored in Cozo. It should not create
markdown memory files.

## Workflow

Generate proposals and memory candidates from the persisted ledger:

```bash
archon agent evolve generate --agent reviewer
archon agent evolve list --agent reviewer
archon agent evolve memory-candidates --agent reviewer
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

## Memory Promotion

Memory candidates are promoted explicitly into the Archon memory graph:

```bash
archon agent evolve memory-promote memory-promotion-123 --dry-run
archon agent evolve memory-promote memory-promotion-123 --min-score 0.85
```

Promotion stores a `Correction` memory in the Cozo-backed memory graph with
agent, candidate, and evidence tags. It does not write `MEMORY.md` or other
memory files.

## Guardrails

Permission changes remain governed proposals. Parent session mode, sandbox
policy, subagent deny lists, and dangerous-bypass guards stay authoritative.
Evolved profiles may narrow behavior automatically, but risky permission or
provider identity changes require explicit approval.
