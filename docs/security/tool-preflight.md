# Tool Preflight

Tool execution passes through a single policy shape before dispatch:

```text
PermissionChecker -> plan-mode gate -> hooks -> sandbox backend -> tool dispatch
```

The exact runtime can mutate inputs through hooks before sandbox checks, so
sandbox decisions are made against the final tool input.

## Permission Rules

`PermissionChecker` deny rules are authoritative before mode convenience.
`dontAsk`, `auto`, and `bypassPermissions` do not defeat explicit deny rules.
`bypassPermissions` also requires the dangerous-bypass CLI guard in session
surfaces that expose it.

## Sandbox Rules

Sandbox policy cannot grant tools. It can only allow the already-permitted tool
to continue, deny it, or route Bash through a backend such as Docker, SSH, or
OpenShell. OpenShell routes when selected and configured safely; otherwise it
fails closed without host-shell fallback.

## Provider Independence

Provider choice does not decide permissions. Anthropic spoofing, Codex runtime
strategy, OpenAI-compatible routing, and local providers all share the same
preflight contract.

## Audit

Sandbox session and runtime decisions are persisted in Cozo with redacted
metadata. Denied or failed sandbox events can become governed learning evidence,
but they must not become automatic permission grants.
