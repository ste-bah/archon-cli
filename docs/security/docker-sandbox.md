# Docker Sandbox

The Docker backend provides local process isolation for Bash when
`sandbox.backend = "docker"` and `sandbox.docker.enabled = true`.

Install host Docker dependencies with:

```bash
sudo scripts/install-system-deps.sh --with-docker
archon sandbox doctor --backend docker
```

## Default Posture

Docker runs with:

- `--cap-drop ALL`
- `--security-opt no-new-privileges`
- no Docker socket mount
- no broad home mount
- no privileged mode
- configured CPU and memory limits
- network disabled or explicitly configured

The workspace is read-only by default. `workspace_access = "rw"` mounts it
read-write. `workspace_access = "scratch"` keeps the workspace read-only and
adds ephemeral `/scratch`.

On Unix the container also runs as **your** user, not as root. `--cap-drop ALL`
removes `CAP_DAC_OVERRIDE`, and without it a container root cannot write
through a bind mount it does not own — which on Linux is every ordinary
checkout, so `workspace_access = "rw"` did not actually permit writes. Running
as the host user fixes that and is the stronger posture besides: nothing in the
container is root, and files it creates in the workspace belong to you rather
than arriving owned by root. Windows passes no user; Docker Desktop translates
ownership itself.

The default `sandbox.mode = "risky"` routes Bash through Docker while leaving
normal host-side coding tools under permission preflight. Use
`sandbox.mode = "all"` only for strict sessions where unsupported host-side
tools should be blocked.

## Container Lifetime

`sandbox.scope` decides how long one container lives, and Docker honours all
three values:

- `session` (default) and `turn` hold a container open and run each command with
  `docker exec`. Everything outside the workspace mount — `~/.cargo/registry`,
  `~/.npm`, pip wheels, apt lists, `/tmp`, `/scratch` — survives between
  commands, which is the whole point: a `docker run --rm` per command destroyed
  every build cache each time, so a sandboxed `cargo build` re-downloaded its
  dependency graph forever. Measured here: 214ms per `docker run --rm` against
  57ms per `docker exec`.
- `tool` builds and destroys a container per command, which is what the backend
  did for every scope before this existed.

A held container is keyed by **working directory** as well as by scope, so a
worktree-isolated subagent never shares one with its parent. Held containers are
identical to per-command ones in isolation — same mount, same `--cap-drop ALL`,
same limits, same network mode — because the same code builds both argument
lists. Per-command environment moves from `--env` on `run` to `--env` on `exec`,
under the same allowlist and the same credential filter.

### Cleanup

Held containers carry `archon.sandbox=1`, `archon.sandbox.owner`,
`archon.sandbox.pid` and `archon.sandbox.scope` labels. Three mechanisms remove
them; see [Sandboxing](sandboxing.md#cleaning-up-held-containers) for why there
are three. The one that holds unconditionally is
`sandbox.docker.container_max_age_secs` (default 4h, minimum 60s): it is the
container's own PID 1 and `--rm` is set, so the container stops and is removed
at that age even if Archon was SIGKILLed and never restarted. Keep it well above
any Bash timeout — a command still running at that age dies with the container.

To find anything left behind by hand:

```bash
docker ps -a --filter label=archon.sandbox=1
```

## Workspace Paths

The workspace is bind-mounted, so the container and the host hold the same
bytes. What differs is the path: Bash runs with `--workdir /workspace`, so
everything it prints — a compiler error, `find` output, a stack trace — names
paths under `/workspace`.

Those paths resolve. Handing `/workspace/src/main.rs` to `Read` reads the same
file the container just wrote, and `Glob` returns container paths so they can be
pasted straight into a Bash command. Host paths keep working unchanged.

Two paths are refused rather than resolved:

- A `/workspace/...` path containing `..` that would climb out of the mount.
  Escaping the mount is the one thing the mount exists to prevent.
- Anything under `/scratch`. That tmpfs exists only inside the container and is
  discarded with it, so it has no host file at all. The error says so rather
  than reporting a missing file.

## Writable Paths

Use relative `docker.writable_paths` when a mostly read-only workspace needs a
specific writable subpath. Absolute paths, parent traversal, commas, and NUL are
rejected.

## Commands

```bash
archon sandbox status --verbose
archon sandbox explain --backend docker
archon sandbox doctor --backend docker
```

Doctor is detect-only. Actual Bash routing happens through the runtime sandbox
backend during tool execution.

For scenario-driven setup examples, see
[Sandbox real-world use cases](../cookbook/sandbox-real-world-use-cases.md).
