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
