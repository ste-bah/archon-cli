# Provider Auth Profiles

Auth profiles are durable provider credential metadata stored in Cozo. They
record ordering, cooldowns, health, source, and redacted display information;
they do not store raw tokens in provider status output.

## Commands

```bash
archon providers profiles import
archon providers profiles list
archon providers profiles inspect <profile-id>
archon providers profiles select anthropic --auth-kind oauth
archon providers profiles cooldown-clear <profile-id>
```

## Selection

The selector prefers healthy eligible profiles, skips disabled or cooled-down
profiles, honors requested auth kinds, and reports skip reasons. Preferred
profiles are used only when still healthy.

## Compatibility

Anthropic and Codex profiles can both exist on the same machine. Codex OAuth
does not trigger Anthropic spoof identity. Anthropic OAuth keeps Claude Code
spoofing host-side.

## Evolution Guardrail

Agent evolution may report provider-profile issues as evidence, but evolved
profiles must not silently switch provider identity or copy OAuth credentials.
Provider-identity-impacting proposals are high risk and require review, approval,
and shadow promotion before activation.
