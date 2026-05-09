# OpenAI-Compatible Providers

OpenAI-compatible providers use Archon's generic provider-neutral runtime path.
They are not routed through Codex app-server strategy and do not inherit
Anthropic Claude Code spoofing.

## Expected Configuration

Configure the provider with its own endpoint, model, and API key according to
the provider's compatibility surface. Auth profile selection and runtime status
are persisted in Cozo when durable evidence is produced.

Useful checks:

```bash
archon providers status
archon providers report
archon providers profiles list --provider openai-compatible
```

## Fallback

If a generic provider cannot be constructed, Archon may use a legacy fallback
only where that compatibility path exists and policy allows it. Fallbacks must
emit provider runtime events with redacted metadata.

## Permissions And Tools

Provider choice does not decide tool access. The same permission mode,
`PermissionChecker` rules, sandbox backend, and subagent deny lists apply across
OpenAI-compatible, Anthropic, Codex, cloud, and local providers.
