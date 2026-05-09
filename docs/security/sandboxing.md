# Sandboxing

Archon's sandbox path is fail-closed. Permission mode, tool policy, and sandbox
backend selection are separate checks; provider spoofing and provider auth do
not change permission decisions.

## Commands

```bash
archon sandbox status --verbose
archon sandbox explain --backend docker
archon sandbox doctor --backend openshell
archon sandbox test --backend docker
```

## Backends

`logical` keeps the existing permission gate behavior.

`docker` can route Bash through a container when selected. It avoids mounting
host credentials, the Docker socket, or privileged host paths by default, and
uses the configured resource and network policy.

`ssh` and `openshell` are detect-only/fail-closed in this slice. If selected
for Bash execution before a real transport is implemented, they return an
error instead of falling back to the host shell.

`archon sandbox status --verbose` shows backend-specific safety knobs, including
Docker host-mount settings and OpenShell provider-injection/host-shell-fallback
flags. `archon sandbox doctor --backend <name>` is also recorded as a redacted
Cozo sandbox runtime event.

Interactive sessions wrap the selected sandbox backend with a Cozo audit layer.
Tool checks and backend Bash execution decisions are recorded as
`sandbox_runtime_events`, and each session creates a `sandbox_sessions` row with
redacted transport details. Commands, environment values, and workspace paths
are not stored in the audit payload. Denied or failed sandbox decisions also
write an agent performance ledger signal so governed evolution can see repeated
sandbox failures without learning around the isolation boundary.

## OpenShell Policy

OpenShell defaults are deliberately conservative:

```toml
provider_injection = false
host_shell_fallback = false
workspace_mode = "mirror"
```

Future OpenShell execution must use the OpenShell transport or configured
gateway. It must not inject provider credentials into the sandbox, and it must
not silently execute directly on the host.
