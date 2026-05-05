# v0.1.45 Provider-Parity Local Transcript

Captured from
`/home/unixdude/Archon-projects/archon-cli-worktrees/provider-parity-production-polish`
on 2026-05-05. Token values were not printed.

## Version

```text
$ ./target/debug/archon --version
archon 0.1.45 (3008321)
```

## Local Auth Status

```text
$ ./target/debug/archon auth status
Anthropic (Claude)
  Status:        authenticated
  Token expires: 2026-05-05 21:24 UTC
  Subscription:  max

Codex (OpenAI ChatGPT subscription)
  Status:           authenticated as account ***...1e9a
  Token expires:    2026-05-14 17:49 UTC
  Spoof identity:   from fetched manifest
    originator:     openclaw
    user-agent:     openclaw/2026.5.1-beta.2
    client-id:      app_EMoamEEZ73...
    openai-beta:    responses=experimental
  Manifest:         https://raw.githubusercontent.com/ste-bah/archon-cli/main/crates/archon-llm/resources/codex-compat.json
  Provider:         enabled (set ARCHON_CODEX_DISABLED=1 to disable)
```

## Provider Doctor

```text
$ ./target/debug/archon providers doctor --live
Provider doctor (local checks + live endpoint reachability)

Credentials file: present
Anthropic OAuth:  present
Codex OAuth:     present
ANTHROPIC_API_KEY env: missing
Anthropic base URL: default
Proxy env:       unset
Anthropic spoof identity: active for Claude OAuth credential file
Codex spoof identity: loaded from bundled/config/env spoof identity at runtime

Capability source of truth: `archon providers capabilities` or `/providers capabilities`
Live provider pings:
  Anthropic ok: endpoint reachable (api.anthropic.com:443)
  Codex     ok: endpoint reachable (chatgpt.com:443)
Remediation:
  - Capability mismatch: run `archon providers capabilities` before using a provider on pipelines/subagents.
```

## Codex Live Chat Marker

```text
$ ./target/debug/archon chat --provider openai-codex --no-stream --max-tokens 16 "Say ARCHON_CODEX_PARITY_OK"
ARCHON_CODEX_PARITY_OK
```

## Notes

- This transcript proves local auth discovery, provider diagnostics, endpoint
  reachability, and a minimal live Codex chat request.
- It is not the expensive full Phase 8 pipeline/TUI transcript. Use
  `docs/maintenance/provider-smoke.md` and `docs/maintenance/codex-smoke.md`
  for the full manual operator checklist.
