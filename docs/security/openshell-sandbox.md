# OpenShell Sandbox

OpenShell support is conservative in this release slice. Configuration, status,
explain, doctor, and Bash execution routing exist, with provider injection and
host shell fallback disabled by default.

## Defaults

```toml
[sandbox.openshell]
enabled = false
binary = "openshell"
workspace_mode = "upload"
gateway = "openshell"
provider_injection = false
host_shell_fallback = false
```

## Host Requirements

OpenShell follows NVIDIA's current support matrix: Debian/Ubuntu Linux on
x86_64/aarch64, macOS Apple Silicon, and Windows WSL2 as experimental. Docker
Desktop or Docker Engine must be installed and running before OpenShell commands
are used. See the NVIDIA OpenShell
[Quickstart](https://docs.nvidia.com/openshell/get-started/quickstart) and
[Support Matrix](https://docs.nvidia.com/openshell/reference/support-matrix).
Use `scripts/install-system-deps.sh --with-openshell --setup-openshell-gateway`
to install OpenShell and start/verify the local gateway as your normal user.

`upload` mode stages the current working directory into `/sandbox/<basename>`
before running Bash. This is the safe default for macOS external volumes such as
`/Volumes/Externalwork/...`, because those host paths do not exist inside the
OpenShell sandbox. `remote` mode runs from `remote_workdir` or `/sandbox`.
`mirror` mode is only for environments where the same absolute path exists
inside the OpenShell runtime.

## Provider Routing

Provider injection stays disabled by default. Anthropic Claude Code spoofing,
Codex OAuth, and provider auth all remain host-side Archon provider runtime
behavior. OpenShell must not receive provider credentials or generated memory
stores unless a separate audited profile explicitly allows that in a later
security-reviewed change.
Archon does not pass OpenShell `--provider` flags and strips common provider
credential environment variables from the OpenShell CLI process before launch.

## Fail-Closed Behavior

If OpenShell is disabled, missing, configured with provider injection, or
configured with host shell fallback, Bash execution returns an error stating that
no host shell fallback was used. When enabled and safe, Bash routes through:

```bash
openshell sandbox create --no-keep --upload '<workdir>:/sandbox' -- /bin/bash -lc '<command>'
```

Configured policies are passed with `--policy`. Configured providers are ignored
while `provider_injection = false`.

With the default `sandbox.mode = "risky"`, OpenShell applies to Bash/Shell
execution only. Normal host-side coding tools such as `Write` and `Edit` still
go through Archon's permission preflight. Set `sandbox.mode = "all"` only when
you want unsupported host-side tools to be blocked.

```bash
archon sandbox doctor --backend openshell
archon sandbox explain --backend openshell
```

For scenario-driven setup examples, see
[Sandbox real-world use cases](../cookbook/sandbox-real-world-use-cases.md).
