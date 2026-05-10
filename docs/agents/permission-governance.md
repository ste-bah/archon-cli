# Agent Permission Governance

Agent evolution can suggest permission-related improvements, but it cannot
silently widen tool access.

## Review Flow

```bash
archon agent evolve list --agent reviewer
archon agent evolve permissions agent-evo-prop-123
archon agent evolve approve agent-evo-prop-123
archon agent evolve shadow agent-evo-prop-123 --task-set regression-suite
archon agent evolve apply agent-evo-prop-123 --activate
```

Permission proposals show structured diffs for review. Repeated denials and
sandbox failures can become evidence, but they are not automatic grants.

## Hard Rules

- Parent session restrictions win.
- `bypassPermissions` is critical risk.
- `dontAsk`, `auto`, and `acceptEdits` are permission expansion when introduced
  by an evolved profile.
- Subagent control-tool deny lists stay authoritative.
- Sandbox policy cannot override `PermissionChecker` deny rules.
- Provider spoofing never implies permission bypass.

## Activation

Permission-impacting proposals require explicit approval. Activation additionally
requires the latest Cozo shadow evaluation to be `promote` with zero regressions.
Applying without activation can stage the version for inspection.
