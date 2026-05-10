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

Host Docker/OpenShell dependencies are optional. Install them with
`scripts/install-system-deps.sh --with-docker`, `--with-openshell`, or
`--with-sandbox`; then enable the matching backend in `[sandbox]`.

By default, `sandbox.mode = "risky"` routes shell execution through the selected
real backend but does not break normal host-side coding tools. `Write`, `Edit`,
`WebFetch`, and similar tools still pass through Archon's permission preflight.
Set `sandbox.mode = "all"` only when you want strict backend compatibility and
are willing to block unsupported host-side mutation, network, and agent-spawn
tools.

## Backends

`logical` keeps the existing permission gate behavior.

`docker` can route Bash through a container when selected. It avoids mounting
host credentials, the Docker socket, or privileged host paths by default, and
uses the configured resource and network policy. The workspace is mounted
read-only unless `workspace_access = "rw"` is configured; teams can keep the
workspace read-only and expose only explicit relative `docker.writable_paths`.
`workspace_access = "scratch"` adds an ephemeral `/scratch` mount without
loosening the workspace.

`ssh` can route Bash through a configured remote target when selected. Remote
mode requires an explicit `ssh.remote_workdir`; mirror mode assumes the active
workspace path exists on the remote target. SSH does not forward provider
credentials, SSH agents, Git credentials, or arbitrary environment values, and
it refuses disabled host-key checking or host-shell fallback.

`openshell` can route Bash through `openshell sandbox create --no-keep --` when
selected. Archon does not pass `--provider`, does not forward request
environment values, strips common provider credential variables from the
OpenShell CLI process, and sets `OPENSHELL_GATEWAY` only when an explicit
gateway is configured. `workspace_mode = "upload"` stages the active workdir
into `/sandbox/<basename>` before Bash, which avoids macOS external-volume paths
such as `/Volumes/...` leaking into the sandbox. `remote` runs from
`remote_workdir` or `/sandbox`; `mirror` assumes the active workspace path is
visible inside the sandbox runtime.

`archon sandbox status --verbose` shows backend-specific safety knobs, including
Docker host-mount settings and OpenShell provider-injection/host-shell-fallback
flags. `archon sandbox explain --backend <name>` expands that into the
permission flow, mount/workspace policy, network policy, and credential-redaction
posture before any command is run. `archon sandbox doctor --backend <name>` is
also recorded as a redacted Cozo sandbox runtime event.

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
workspace_mode = "upload"
gateway = "openshell"
```

OpenShell execution must use the OpenShell transport or configured gateway. It
must not inject provider credentials into the sandbox, and it must not silently
execute directly on the host.

See also:

- [Sandbox real-world use cases](../cookbook/sandbox-real-world-use-cases.md)
- [Docker sandbox](docker-sandbox.md)
- [SSH sandbox](ssh-sandbox.md)
- [OpenShell sandbox](openshell-sandbox.md)
- [Tool preflight](tool-preflight.md)
