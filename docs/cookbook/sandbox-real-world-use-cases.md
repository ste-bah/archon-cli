# Sandbox real-world use cases

This cookbook is the plain-English guide to Docker, SSH, and OpenShell
sandboxing in Archon.

The short version:

- Archon agents stay on the host.
- Provider auth, Claude Code spoofing, Codex routing, memory, and governance stay
  on the host.
- The risky shell hand can be routed through Docker, SSH, or OpenShell.
- The default real-backend mode is built for normal coding: Bash is sandboxed,
  while `Write`, `Edit`, and other host-side tools still go through permission
  preflight.

## Choose The Right Posture

| Situation | Recommended backend | Why |
|---|---|---|
| Normal coding in your own repo | `disabled` or real backend with `mode = "risky"` | Host-side `Write` and `Edit` keep working; Bash can be isolated if you choose a real backend. |
| Reviewing unknown scripts | `docker` with `mode = "risky"` and `workspace_access = "scratch"` | Bash can inspect the repo and write only to `/scratch`. |
| Running tests from a dependency-heavy repo | `docker` with `mode = "risky"` and selected `writable_paths` | Keeps shell execution contained but lets build caches or output dirs work. |
| Strict read-only investigation | `docker` or `openshell` with `mode = "all"` | Unsupported host-side mutation, network, and agent-spawn tools are blocked. |
| Team remote execution target | `ssh` with `workspace_mode = "remote"` | Bash runs on an explicit remote sandbox host. |
| Mediated OpenShell environment | `openshell` with provider injection disabled | OpenShell owns the sandbox lifecycle; Archon keeps provider credentials host-side. |

## What Actually Moves

Docker, SSH, and OpenShell are not whole-agent containers. They are tool
execution backends.

When a real backend is enabled and `sandbox.mode = "risky"`:

| Tool | Behavior |
|---|---|
| `Bash` / `Shell` | Routed through the selected backend. |
| `Write` / `Edit` / `NotebookEdit` | Host-side, still governed by permission preflight. |
| `Read` / `Grep` / `Glob` | Host-side/read-oriented, still governed by normal checks. |
| `WebFetch` / `WebSearch` | Host-side, still governed by permission preflight. |
| Subagent Bash | Inherits the parent sandbox backend and routes the same way. |

When `sandbox.mode = "all"`, real backends become strict compatibility gates.
Unsupported host-side mutation, network, and agent-spawn tools are blocked
instead of falling back to the host.

PowerShell is not routed through Docker/OpenShell yet. In real-backend shell
modes, Archon blocks it rather than silently running it outside the sandbox.

## First-Time Setup

Install the host dependencies only when you plan to use the backend:

```bash
sudo scripts/install-system-deps.sh --with-docker
sudo scripts/install-system-deps.sh --with-openshell
sudo scripts/install-system-deps.sh --with-sandbox
```

For OpenShell, also set up the local gateway as your normal user after Docker is
running:

```bash
scripts/install-system-deps.sh --with-openshell --setup-openshell-gateway
```

That command refreshes OpenShell through NVIDIA's official install script and
starts the gateway only if `openshell status` reports no active gateway. Do not
run the gateway setup under `sudo`; OpenShell stores active gateway metadata in
the current user's profile.

Run diagnostics before enabling anything:

```bash
archon sandbox status --verbose
archon sandbox doctor --backend docker
archon sandbox doctor --backend openshell
archon sandbox explain --backend docker --tool Bash --command "cargo test"
archon sandbox test --backend docker
```

`sandbox test` and `sandbox doctor` are detect-only. They validate config and
tool availability without running an untrusted command.

For Docker, the configured image must already exist locally because Archon uses
`--pull never` when it executes sandboxed Bash:

```bash
docker pull ubuntu:24.04
```

The image also needs the tools your command expects. `ubuntu:24.04` is a safe
baseline, not a full project toolchain. For Rust tests, Node builds, Python
linters, or package-manager commands, build a project image first and point
`sandbox.docker.image` at that image.

For OpenShell, Docker Desktop or Docker Engine must be running first. OpenShell
support follows NVIDIA's current support matrix. See
[OpenShell sandbox](../security/openshell-sandbox.md) for host requirements.
If `openshell sandbox create` reports `No active gateway`, rerun the installer
with `--setup-openshell-gateway`. On macOS the installer uses the Homebrew
service for `nvidia/openshell/openshell`; on Linux it enables and restarts the
user `openshell-gateway` systemd service, then registers
`https://127.0.0.1:17670` as the local gateway. Older CLI builds with
`openshell gateway start` are supported as a fallback.

## Use Case 1: Normal Coding, Safer Shell

Use this when you want agents to edit files normally, but you do not want their
shell commands to run directly on the host.

```toml
[sandbox]
backend = "docker"
mode = "risky"
workspace_access = "ro"

[sandbox.docker]
enabled = true
image = "ubuntu:24.04"
network = "disabled"
```

What happens:

- `Write` and `Edit` still modify host files after permission preflight.
- `Bash` runs inside Docker.
- Bash cannot write the workspace because `workspace_access = "ro"`.
- If the agent needs to change source files, it should use `Edit`/`Write`, not
  shell redirection.

This is the safest default for coding agents that need normal file edits.

## Use Case 2: Let Tests Write Build Output

Some test commands need `target/`, `.pytest_cache/`, or similar output
directories. Keep the workspace read-only and allow only specific paths:

```toml
[sandbox]
backend = "docker"
mode = "risky"
workspace_access = "ro"

[sandbox.docker]
enabled = true
image = "archon-rust-sandbox:local"
network = "disabled"
writable_paths = ["target", ".pytest_cache", "tmp"]
```

What happens:

- Bash runs in Docker.
- Most of the workspace is read-only.
- The listed relative paths are mounted writable.
- Absolute paths and `..` traversal are rejected.

Create the writable paths intentionally before first use if you care about host
ownership and permissions:

```bash
mkdir -p target .pytest_cache tmp
```

Use an image that already contains the required toolchain. Example:

```dockerfile
FROM rust:1.85-bookworm
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev git \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /workspace
```

```bash
docker build -t archon-rust-sandbox:local -f Dockerfile.sandbox .
```

Keep `network = "disabled"` when the image already has what it needs. Set
`network = "enabled"` only for sessions where you explicitly accept package or
dependency downloads from inside the sandbox.

Use this for “run the test suite” or “generate temporary artifacts” work.

## Use Case 3: Scratch-Only Unknown Script Review

Use this when the command should inspect the project but not write into it:

```toml
[sandbox]
backend = "docker"
mode = "risky"
workspace_access = "scratch"

[sandbox.docker]
enabled = true
image = "ubuntu:24.04"
network = "disabled"
```

The workspace is mounted read-only and `/scratch` is writable. Archon also sets:

```bash
ARCHON_SANDBOX_SCRATCH=/scratch
```

Ask the agent to write any generated output to `/scratch`. Host-side `Edit` and
`Write` are still available in `mode = "risky"`, so keep the permission mode at
`default` or `plan` if you want eyes on every host mutation.

## Use Case 4: Strict Read-Only Investigation

Use strict mode when you want a session that refuses unsupported host-side tools:

```toml
[sandbox]
backend = "docker"
mode = "all"
workspace_access = "scratch"

[sandbox.docker]
enabled = true
image = "ubuntu:24.04"
network = "disabled"
```

What happens:

- `Bash` routes to Docker.
- `Write`, `Edit`, `WebFetch`, `WebSearch`, and agent-spawn tools are blocked by
  the real backend compatibility gate.
- Read-oriented tools still work.

This is useful for audit/review sessions, not for normal implementation work.

## Use Case 5: OpenShell Mediated Execution

Use OpenShell when you want Archon Bash commands to go through OpenShell's
sandbox lifecycle.

```toml
[sandbox]
backend = "openshell"
mode = "risky"

[sandbox.openshell]
enabled = true
binary = "openshell"
workspace_mode = "mirror"
provider_injection = false
host_shell_fallback = false
```

What happens:

- Bash routes through `openshell sandbox create --no-keep --`.
- Claude Code spoofing, Codex OAuth, provider auth, and memory stay host-side.
- Archon strips common provider credentials from the OpenShell process.
- OpenShell provider entries are ignored while `provider_injection = false`.

`workspace_mode = "mirror"` assumes the active host path is visible inside the
OpenShell runtime. Use `remote` only when you have a configured gateway:

```toml
[sandbox.openshell]
enabled = true
workspace_mode = "remote"
gateway = "team-gateway"
provider_injection = false
host_shell_fallback = false
```

OpenShell workspace mutability is governed by the OpenShell environment and any
configured OpenShell policy. Archon still controls whether non-shell host tools
continue through permission preflight (`risky`) or are blocked (`all`).

## Use Case 6: Remote SSH Sandbox

Use SSH when the sandbox is a remote machine:

```toml
[sandbox]
backend = "ssh"
mode = "risky"

[sandbox.ssh]
enabled = true
host = "sandbox.example"
user = "archon"
workspace_mode = "remote"
remote_workdir = "/srv/archon/workspace"
host_key_checking = true
host_shell_fallback = false
```

What happens:

- Bash runs on the configured remote host.
- Provider credentials, SSH agents, Git credentials, and arbitrary environment
  values are not forwarded by default.
- Host shell fallback is refused.

Use SSH for team-managed execution machines or remote test rigs.

## Avoid These Traps

Do not treat `/sandbox on` as the Docker/OpenShell switch. The TUI slash command
is the logical policy gate. Real backends are selected in `config.toml`.

Do not enable `sandbox.mode = "all"` for day-to-day coding unless you want
`Write` and `Edit` blocked.

Do not set OpenShell `provider_injection = true` for normal Archon use. That
would move provider behavior into OpenShell and risks breaking Claude Code spoof
compatibility. Archon intentionally keeps provider identity host-side.

Do not expect Docker to pull images during agent execution. Pull or build the
image first.

Do not rely on shell redirection for source edits when Docker has
`workspace_access = "ro"` or `"scratch"`. Use `Edit`/`Write` so Archon's normal
permission and checkpoint behavior remains in charge.

## Quick Decision Rules

Use `disabled` when you trust the repo and want the least friction.

Use `docker` + `risky` for safer command execution during normal coding.

Use `docker` + `scratch` for unknown commands that may need temporary output.

Use `docker` + `all` for audits where host mutation should be blocked.

Use `openshell` + `risky` when you want OpenShell-mediated Bash while preserving
Archon's provider and memory runtime.

Use `ssh` when the execution environment should be a separate remote host.

## See Also

- [Sandboxing](../security/sandboxing.md)
- [Docker sandbox](../security/docker-sandbox.md)
- [OpenShell sandbox](../security/openshell-sandbox.md)
- [SSH sandbox](../security/ssh-sandbox.md)
- [Tool preflight](../security/tool-preflight.md)
- [Configuration](../reference/config.md#sandbox)
