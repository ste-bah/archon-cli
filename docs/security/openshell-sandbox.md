# OpenShell Sandbox

OpenShell support is conservative in this release slice. Configuration, status,
explain, doctor, and Bash execution routing exist, with provider injection and
host shell fallback disabled by default.

## Defaults

```toml
[sandbox.openshell]
enabled = false
binary = "openshell"
workspace_mode = "mirror"
provider_injection = false
host_shell_fallback = false
```

`mirror` mode treats the local Archon workspace as canonical and assumes that
path is visible inside the OpenShell sandbox runtime. `remote` mode must
configure a gateway and runs commands from `/sandbox`.

## Provider Routing

Provider injection stays disabled by default. Anthropic Claude Code spoofing,
Codex OAuth, and provider auth all remain host-side Archon provider runtime
behavior. OpenShell must not receive provider credentials or generated memory
stores unless a separate audited profile explicitly allows it in a future slice.
Archon does not pass OpenShell `--provider` flags and strips common provider
credential environment variables from the OpenShell CLI process before launch.

## Fail-Closed Behavior

If OpenShell is disabled, missing, configured with provider injection, or
configured with host shell fallback, Bash execution returns an error stating that
no host shell fallback was used. When enabled and safe, Bash routes through:

```bash
openshell sandbox create --no-keep -- /bin/bash -lc '<command>'
```

Configured policies are passed with `--policy`. Configured providers are ignored
while `provider_injection = false`.

```bash
archon sandbox doctor --backend openshell
archon sandbox explain --backend openshell
```
