# OpenShell Sandbox

OpenShell support is conservative in this release slice. Configuration, status,
explain, doctor, and fail-closed backend behavior exist; execution transport is
still disabled.

## Defaults

```toml
[sandbox.openshell]
enabled = false
binary = "openshell"
workspace_mode = "mirror"
provider_injection = false
host_shell_fallback = false
```

`mirror` mode treats the local Archon workspace as canonical. `remote` mode must
configure a gateway and remains explicit in status and explain output.

## Provider Routing

Provider injection stays disabled by default. Anthropic Claude Code spoofing,
Codex OAuth, and provider auth all remain host-side Archon provider runtime
behavior. OpenShell must not receive provider credentials or generated memory
stores unless a separate audited profile explicitly allows it in a future slice.

## Fail-Closed Behavior

If selected for Bash execution today, OpenShell returns an error stating that the
transport is not implemented and that no host shell fallback was used.

```bash
archon sandbox doctor --backend openshell
archon sandbox explain --backend openshell
```
