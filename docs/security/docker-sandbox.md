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

### Resource limits are now shared

`--memory`, `--cpus` and `--pids-limit` are per container, so consolidating
containers consolidates the limits. **A fan-out that fitted before can stop
fitting.** Ten subagents in one working directory used to get ten containers and
ten times the budget; they now share one container's `--memory 2g` and
`--pids-limit 256`. Raise `memory_limit`/`cpu_limit` to cover the whole fan-out,
or set `sandbox.scope = "tool"` for a container per command. Separate worktrees
still get separate containers, and separate limits with them.

### Cleanup

**Every** container Archon starts is labelled — held, per-command and terminal
alike:

```bash
docker ps -a --filter label=archon.sandbox=1
docker ps -a --filter label=archon.sandbox.kind=terminal
```

`archon.sandbox=1`, `archon.sandbox.owner`, `archon.sandbox.pid` and
`archon.sandbox.kind` (`held` | `command` | `terminal`). The labels go on where
the isolation arguments are built, so no creation path can produce an unfindable
container by omission.

Three mechanisms remove them; see
[Sandboxing](sandboxing.md#cleaning-up-containers) for why there are three. The
one that holds unconditionally is `sandbox.docker.container_max_age_secs`
(default 4h, minimum 60s), enforced from inside the container — `sleep` as PID 1,
or `timeout --signal=KILL` around a terminal's shell — with `--rm` set, so the
container stops and is removed at that age even if Archon was SIGKILLed and never
restarted. Keep it well above any Bash timeout: a command or shell still running
at that age dies with the container.

Reaping runs once per docker backend, before its first sandboxed command. Two
consequences worth knowing:

- **Under the workflow CLI, `Drop` never runs.** The subagent executor is
  installed process-globally and holds the backend for the life of the process,
  so a workflow run's containers are collected by the age bound, or by the next
  Archon that starts and reaps them — not at the end of the run. Under a 4h
  default that is a 4h window.
- A session that only ever opens a terminal never runs a `Bash` command, so it
  never triggers reaping. Its own containers are still labelled and still
  age-bounded.

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
